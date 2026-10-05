//! [`Source`]: one watch (one API, one namespace or the whole cluster) of an events feed.
//!
//! The same `kube::runtime::watcher` as a reflector feed, with the same backoff, error
//! mapping and uncompressed watch client, but **no reflector store**: each object becomes a
//! domain [`Event`] as it arrives and is passed on, so the only copy a feed keeps is the
//! bounded [`EventRing`](super::ring::EventRing).
//!
//! | watcher event | message to the pump |
//! |---|---|
//! | `Init` | [`Msg::ListStart`] |
//! | `InitApply(o)` | [`Msg::ListItem`] |
//! | `InitDone` | [`Msg::ListDone`] |
//! | `Apply(o)` / `Delete(o)` | [`Msg::Apply`] / [`Msg::Delete`] |
//! | retryable error | [`Msg::Error`], then back off and continue |
//! | other error | [`Msg::Fatal`], then the task ends |
//!
//! Objects that do not map to an event, and events about another object than
//! `regarding_uid`, are dropped here (the first are counted in the feed's `skipped`).
//!
//! # Fallbacks
//!
//! Before the first list completes, an HTTP 400 or 422 means the server refused something
//! optional: a streaming list first (as in `feed`), then the UID field selector. The watch
//! restarts without it, and the client-side UID filter keeps the result correct.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use futures::StreamExt;
use kube::Api;
use kube::core::{ApiResource, DynamicObject};
use kube::runtime::watcher::{self, Event as Watched, InitialListStrategy};
use oxikube_domain::OxiError;
use oxikube_domain::event::Event;
use oxikube_domain::ids::ClusterId;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};
use tracing::debug;

use super::config::EventApi;
use super::handle::Counters;
use crate::feed::FeedConfig;
use crate::feed::FeedState;
use crate::feed::error::feed_error;
use crate::feed::source::{self, EventStream, is_streaming};
use crate::feed::transport::Accepted;
use crate::resources::dynamic_json;

/// What a [`Source`] tells the pump.
pub(super) enum Msg {
    /// A (re)list starts.
    ListStart,
    /// One event of the list in progress.
    ListItem(Event),
    /// The list in progress is complete.
    ListDone,
    /// An event was added or changed.
    Apply(Event),
    /// An event was deleted.
    Delete(Event),
    /// The watch entered this state.
    State(FeedState),
    /// A retryable failure; the watch backs off and continues.
    Error(OxiError),
    /// A non-retryable failure; the watch has stopped.
    Fatal(OxiError),
}

/// A [`Msg`] from the watch numbered `index`.
pub(super) struct Tagged {
    pub(super) index: usize,
    pub(super) msg: Msg,
}

/// The pump has gone away (the feed was dropped).
struct Closed;

/// One watch of a feed.
pub(super) struct Source {
    pub(super) index: usize,
    pub(super) api: EventApi,
    pub(super) kube_api: Api<DynamicObject>,
    pub(super) resource: ApiResource,
    pub(super) accepted: Accepted,
    pub(super) watcher_config: watcher::Config,
    pub(super) config: FeedConfig,
    pub(super) cluster: ClusterId,
    pub(super) regarding_uid: Option<Arc<str>>,
    pub(super) counters: Arc<Counters>,
    pub(super) tx: mpsc::Sender<Tagged>,
}

/// Where the watch is in its list/watch cycle.
struct Cycle {
    synced: bool,
    state: FeedState,
    before_retry: FeedState,
    /// Set once a reconnect got through while retrying.
    settle_at: Option<Instant>,
}

impl Source {
    /// Runs until the pump goes away or a non-retryable error stops the watch.
    pub(super) async fn run(mut self) {
        let _ = self.drive().await;
    }

    fn open(&self) -> EventStream {
        source::events(
            self.kube_api.clone(),
            self.watcher_config.clone(),
            &self.config,
        )
    }

    async fn drive(&mut self) -> Result<(), Closed> {
        let mut stream = self.open();
        let mut cycle = Cycle {
            synced: false,
            state: FeedState::Warming,
            before_retry: FeedState::Warming,
            settle_at: None,
        };
        loop {
            let reconnecting = cycle.state == FeedState::Retrying && cycle.settle_at.is_none();
            let item = tokio::select! {
                item = stream.next() => item,
                Ok(()) = self.accepted.changed(), if reconnecting => {
                    cycle.settle_at = Some(Instant::now() + self.config.retry_settle);
                    continue;
                }
                () = sleep_until(cycle.settle_at.unwrap_or_else(Instant::now)),
                    if cycle.settle_at.is_some() => {
                    cycle.settle_at = None;
                    let back = cycle.before_retry;
                    self.set_state(&mut cycle, back).await?;
                    continue;
                }
            };
            match item {
                // The backoff never gives up, so the watcher never ends on its own.
                None => return Ok(()),
                Some(Err(err)) => {
                    if !cycle.synced && refused(&err) && self.drop_optional_request() {
                        stream = self.open();
                        continue;
                    }
                    self.on_error(&mut cycle, &err).await?;
                }
                Some(Ok(event)) => {
                    cycle.settle_at = None;
                    self.on_event(&mut cycle, event).await?;
                }
            }
        }
    }

    /// After a 400/422 on the first list: stops asking for the streaming list, else for the
    /// field selector. `false` when there is nothing left to drop.
    fn drop_optional_request(&mut self) -> bool {
        let wc = &mut self.watcher_config;
        if is_streaming(wc) {
            debug!(api = ?self.api, "streaming list rejected; using paged lists");
            wc.initial_list_strategy = InitialListStrategy::ListWatch;
            true
        } else if wc.field_selector.take().is_some() {
            debug!(api = ?self.api, "field selector rejected; filtering on the client");
            true
        } else {
            false
        }
    }

    async fn on_error(&mut self, cycle: &mut Cycle, err: &watcher::Error) -> Result<(), Closed> {
        let error = feed_error(err);
        if !error.is_retryable() {
            self.set_state(cycle, FeedState::Stopped).await?;
            return self.send(Msg::Fatal(error)).await;
        }
        debug!(api = ?self.api, error = %error, "events watch failed; backing off");
        if cycle.state != FeedState::Retrying {
            cycle.before_retry = cycle.state;
            self.set_state(cycle, FeedState::Retrying).await?;
        }
        // Only a watch accepted after this error shows the server is back.
        cycle.settle_at = None;
        self.accepted.borrow_and_update();
        self.send(Msg::Error(error)).await
    }

    async fn on_event(
        &self,
        cycle: &mut Cycle,
        event: Watched<DynamicObject>,
    ) -> Result<(), Closed> {
        let next = match &event {
            Watched::Init if cycle.state == FeedState::Retrying => FeedState::Retrying,
            Watched::Init | Watched::InitApply(_) => FeedState::Warming,
            Watched::InitDone | Watched::Apply(_) | Watched::Delete(_) => FeedState::Live,
        };
        match event {
            Watched::Init => self.send(Msg::ListStart).await?,
            Watched::InitApply(o) => {
                if let Some(event) = self.convert(o) {
                    self.send(Msg::ListItem(event)).await?;
                }
            }
            Watched::InitDone => {
                cycle.synced = true;
                self.send(Msg::ListDone).await?;
            }
            Watched::Apply(o) => {
                if let Some(event) = self.convert(o) {
                    self.send(Msg::Apply(event)).await?;
                }
            }
            Watched::Delete(o) => {
                if let Some(event) = self.convert(o) {
                    self.send(Msg::Delete(event)).await?;
                }
            }
        }
        self.set_state(cycle, next).await
    }

    /// The domain event of `object`; `None` when it does not map (counted, never logged with
    /// its content) or is about another object than the one this feed is for.
    fn convert(&self, object: DynamicObject) -> Option<Event> {
        let json = dynamic_json(object, &self.resource, true);
        let event = match Event::from_json(&self.cluster, &json) {
            Ok(event) => event,
            Err(_) => {
                self.counters.skipped.fetch_add(1, Ordering::Relaxed);
                debug!(api = ?self.api, "skipping an event that does not map to the domain type");
                return None;
            }
        };
        match &self.regarding_uid {
            Some(uid) if event.regarding_uid.as_deref() != Some(uid) => None,
            _ => Some(event),
        }
    }

    async fn set_state(&self, cycle: &mut Cycle, state: FeedState) -> Result<(), Closed> {
        if cycle.state == state {
            return Ok(());
        }
        cycle.state = state;
        self.send(Msg::State(state)).await
    }

    async fn send(&self, msg: Msg) -> Result<(), Closed> {
        let index = self.index;
        self.tx
            .send(Tagged { index, msg })
            .await
            .map_err(|_| Closed)
    }
}

/// The server refused the request with HTTP 400 or 422 (an unknown field label, a feature
/// that is off), as opposed to failing.
fn refused(err: &watcher::Error) -> bool {
    use watcher::Error as E;
    match err {
        E::InitialListFailed(kube::Error::Api(status))
        | E::WatchStartFailed(kube::Error::Api(status))
        | E::WatchFailed(kube::Error::Api(status)) => matches!(status.code, 400 | 422),
        E::WatchError(status) => matches!(status.code, 400 | 422),
        _ => false,
    }
}
