//! [`SubWatch`]: one `watcher` + reflector store (one namespace, or the whole cluster),
//! run as a task that reports to the feed's pump.
//!
//! Each watcher event is converted to a domain object once, applied to the reflector store
//! and turned into what the consumer needs:
//!
//! | watcher event | first list | later relist ([`Diff`](super::RelistDelivery::Diff)) | watching |
//! |---|---|---|---|
//! | `Init` | start collecting | start a [`RelistDiff`] | — |
//! | `InitApply(o)` | collect `o` | `Applied` if new or changed | — |
//! | `InitDone` | [`SubEvent::Synced`] with the list | `Deleted` for what vanished | — |
//! | `Apply(o)` / `Delete(o)` | — | — | `Applied(o)` / `Deleted(o)` |
//! | error | [`SubEvent::Error`] (retryable) or [`SubEvent::Fatal`] (then the task ends) | | |

use std::mem;
use std::time::Duration;

use futures::StreamExt;
use kube::Api;
use kube::core::{ApiResource, DynamicObject};
use kube::runtime::reflector::store::Writer;
use kube::runtime::watcher::{self, Event, InitialListStrategy};
use oxikube_domain::{OxiError, Resource};
use oxikube_ports::Delta;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};
use tracing::{debug, warn};

use super::config::{FeedConfig, RelistDelivery};
use super::error::{feed_error, streaming_rejected};
use super::object::FeedObject;
use super::relist::RelistDiff;
use super::source::{self, EventStream};
use super::state::FeedState;

/// What a [`SubWatch`] tells the pump.
pub(super) enum SubEvent {
    /// The first list of watch `index` completed with these objects.
    Synced {
        index: usize,
        objects: Vec<Resource>,
    },
    /// A watch relisted and [`RelistDelivery::Snapshot`] is set: send the stores.
    Resynced,
    /// One change, in order.
    Delta(Delta<Resource>),
    /// Watch `index` entered `state`.
    State { index: usize, state: FeedState },
    /// A retryable failure; the watch backs off and continues.
    Error(OxiError),
    /// A non-retryable failure; the watch has stopped.
    Fatal(OxiError),
}

/// The pump has gone away (the feed was dropped).
struct Closed;

/// One watch and its reflector store.
pub(super) struct SubWatch {
    pub(super) index: usize,
    pub(super) api: Api<DynamicObject>,
    pub(super) resource: ApiResource,
    pub(super) watcher_config: watcher::Config,
    pub(super) config: FeedConfig,
    pub(super) strip_managed_fields: bool,
    pub(super) writer: Writer<FeedObject>,
    pub(super) tx: mpsc::Sender<SubEvent>,
}

/// Where the watch is in its list/watch cycle.
#[derive(Default)]
struct Cycle {
    /// The first list completed.
    synced: bool,
    /// Objects of the first list, collected until `InitDone`.
    initial: Vec<Resource>,
    /// The diff of a later relist in progress.
    relist: Option<RelistDiff>,
    /// Reported state, and the one to return to when a retry settles.
    state: FeedState,
    before_retry: FeedState,
}

impl SubWatch {
    /// Runs until the pump goes away or a non-retryable error stops the watch.
    pub(super) async fn run(mut self) {
        let _ = self.drive().await;
    }

    async fn drive(&mut self) -> Result<(), Closed> {
        let mut stream = self.open();
        let mut cycle = Cycle::default();
        let mut settle_at: Option<Instant> = None;
        let mut fell_back = false;
        loop {
            let item = tokio::select! {
                item = stream.next() => item,
                () = sleep_until(settle_at.unwrap_or_else(Instant::now)), if settle_at.is_some() => {
                    settle_at = None;
                    let back = cycle.before_retry;
                    self.set_state(&mut cycle, back).await?;
                    continue;
                }
            };
            match item {
                // The backoff never gives up, so the watcher never ends on its own.
                None => return Ok(()),
                Some(Err(err)) => {
                    if !cycle.synced && !fell_back && rejected_streaming(&self.watcher_config, &err)
                    {
                        debug!(kind = %self.resource.kind, "streaming list rejected; using paged lists");
                        fell_back = true;
                        self.watcher_config.initial_list_strategy = InitialListStrategy::ListWatch;
                        stream = self.open();
                        continue;
                    }
                    let error = feed_error(&err);
                    if !error.is_retryable() {
                        self.set_state(&mut cycle, FeedState::Stopped).await?;
                        return self.send(SubEvent::Fatal(error)).await;
                    }
                    debug!(kind = %self.resource.kind, error = %error, "watch failed; backing off");
                    if cycle.state != FeedState::Retrying {
                        cycle.before_retry = cycle.state;
                        self.set_state(&mut cycle, FeedState::Retrying).await?;
                    }
                    settle_at = Some(Instant::now() + self.retry_settle());
                    self.send(SubEvent::Error(error)).await?;
                }
                Some(Ok(event)) => {
                    settle_at = None;
                    self.on_event(&mut cycle, event).await?;
                }
            }
        }
    }

    fn open(&self) -> EventStream {
        source::events(self.api.clone(), self.watcher_config.clone(), &self.config)
    }

    fn retry_settle(&self) -> Duration {
        self.config.retry_settle
    }

    async fn on_event(
        &mut self,
        cycle: &mut Cycle,
        event: Event<DynamicObject>,
    ) -> Result<(), Closed> {
        let Some(event) = self.convert(event) else {
            return Ok(());
        };
        let next = match &event {
            Event::Init | Event::InitApply(_) => FeedState::Warming,
            Event::InitDone | Event::Apply(_) | Event::Delete(_) => FeedState::Live,
        };
        let store = self.writer.as_reader();
        let mut out = Vec::new();
        // The writer clones what it stores; the event keeps its own object for the consumer.
        // `InitDone` is applied below, after a relist diff has read the previous state.
        if !matches!(event, Event::InitDone) {
            self.writer.apply_watcher_event(&event);
        }
        let diffing = self.config.relist == RelistDelivery::Diff;
        match event {
            Event::Init if cycle.synced => cycle.relist = diffing.then(RelistDiff::default),
            Event::Init => cycle.initial.clear(),
            Event::InitApply(object) => {
                if !cycle.synced {
                    cycle.initial.push(object.0);
                } else if let Some(diff) = cycle.relist.as_mut() {
                    diff.object(&store, object, &mut out);
                }
            }
            Event::InitDone => {
                if let Some(diff) = cycle.relist.take() {
                    diff.finish(&store, &mut out);
                }
                self.writer.apply_watcher_event(&Event::InitDone);
                if !cycle.synced {
                    cycle.synced = true;
                    let objects = mem::take(&mut cycle.initial);
                    self.send(SubEvent::Synced {
                        index: self.index,
                        objects,
                    })
                    .await?;
                } else if !diffing {
                    self.send(SubEvent::Resynced).await?;
                }
            }
            Event::Apply(object) => out.push(Delta::Applied(object.0)),
            Event::Delete(object) => out.push(Delta::Deleted(object.0)),
        }
        for delta in out {
            self.send(SubEvent::Delta(delta)).await?;
        }
        self.set_state(cycle, next).await
    }

    /// The event with its object converted to a domain object; `None` (logged) when the
    /// object is not valid, so it is skipped.
    fn convert(&self, event: Event<DynamicObject>) -> Option<Event<FeedObject>> {
        let strip = self.strip_managed_fields;
        let one = |object| {
            let converted = FeedObject::convert(object, &self.resource, strip);
            if converted.is_none() {
                warn!(kind = %self.resource.kind, "skipping an invalid object from the watch");
            }
            converted
        };
        Some(match event {
            Event::Init => Event::Init,
            Event::InitDone => Event::InitDone,
            Event::InitApply(o) => Event::InitApply(one(o)?),
            Event::Apply(o) => Event::Apply(one(o)?),
            Event::Delete(o) => Event::Delete(one(o)?),
        })
    }

    async fn set_state(&self, cycle: &mut Cycle, state: FeedState) -> Result<(), Closed> {
        if cycle.state == state {
            return Ok(());
        }
        cycle.state = state;
        self.send(SubEvent::State {
            index: self.index,
            state,
        })
        .await
    }

    async fn send(&self, event: SubEvent) -> Result<(), Closed> {
        self.tx.send(event).await.map_err(|_| Closed)
    }
}

fn rejected_streaming(wc: &watcher::Config, err: &watcher::Error) -> bool {
    source::is_streaming(wc) && streaming_rejected(err)
}
