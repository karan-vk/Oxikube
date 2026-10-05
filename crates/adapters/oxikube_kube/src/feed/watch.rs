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
//!
//! # Retrying
//!
//! A retryable error puts the watch in [`FeedState::Retrying`]; it stays there through the
//! backoff and every failed or still-connecting attempt. It leaves on the first object event
//! (`Init` alone does not count: the watcher emits it before its list request), or once the
//! server has accepted a new watch request ([`Accepted`]) and
//! [`FeedConfig::retry_settle`] has passed since without another error: the reconnected
//! watch is quiet, not broken.

use std::mem;

use futures::StreamExt;
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
use super::source::{EventStream, Target};
use super::state::FeedState;
use super::transport::Accepted;

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
    pub(super) target: Target,
    /// Watch requests of `api` the server accepted.
    pub(super) accepted: Accepted,
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
        // Set once a reconnect got through while retrying.
        let mut settle_at: Option<Instant> = None;
        let mut fell_back = false;
        loop {
            let reconnecting = cycle.state == FeedState::Retrying && settle_at.is_none();
            let item = tokio::select! {
                item = stream.next() => item,
                Ok(()) = self.accepted.changed(), if reconnecting => {
                    settle_at = Some(Instant::now() + self.config.retry_settle);
                    continue;
                }
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
                    // Only a watch accepted after this error shows the server is back.
                    settle_at = None;
                    self.accepted.borrow_and_update();
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
        self.target
            .events(self.watcher_config.clone(), &self.config)
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
            // The watcher restarting its list, before any request: no sign of recovery.
            Event::Init if cycle.state == FeedState::Retrying => FeedState::Retrying,
            Event::Init | Event::InitApply(_) => FeedState::Warming,
            Event::InitDone | Event::Apply(_) | Event::Delete(_) => FeedState::Live,
        };
        // Deltas of a relist diff; live changes are sent straight from their arm.
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
                    diff.object(&self.writer.as_reader(), object, &mut out);
                }
            }
            Event::InitDone => {
                if let Some(diff) = cycle.relist.take() {
                    diff.finish(&self.writer.as_reader(), &mut out);
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
            Event::Apply(object) => self.send(SubEvent::Delta(Delta::Applied(object.0))).await?,
            Event::Delete(object) => self.send(SubEvent::Delta(Delta::Deleted(object.0))).await?,
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
        let partial = self.target.is_metadata();
        let one = |object| {
            let converted = FeedObject::convert(object, &self.resource, strip, partial);
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
    super::source::is_streaming(wc) && streaming_rejected(err)
}
