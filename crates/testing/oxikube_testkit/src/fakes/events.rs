//! [`FakeEventsPort`], the scripted [`FakeEventFeed`] it serves and the [`event`] builder
//! (E14-S01): the cluster's normalised `Event`s without a cluster.
//!
//! A test describes the life of an events feed once (the events listed first, then what is
//! applied or deleted at tick 1, 2, ...) and gets the batches the production watcher hands
//! the `EventService`: one `Restarted` at tick 0, then one batch per tick. Ticks are replayed
//! on the fake's clock, so nothing sleeps and no thread starts.
//!
//! ```
//! use oxikube_testkit::{FakeEventFeed, FakeEventsPort, event};
//!
//! let feed = FakeEventFeed::new()
//!     .initial([event().uid("a").warning().reason("BackOff").build()])
//!     .add(1, event().uid("b").reason("Pulled").build());
//! assert_eq!(feed.last_tick(), 1);
//! let port = FakeEventsPort::new();
//! port.install(feed);
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::OxiResult;
use oxikube_domain::event::{Event, EventType};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::session::WatchScope;
use oxikube_ports::{
    ClockPort, Delta, DeltaBatch, EventApi, EventsFeed, EventsMonitor, EventsPort, EventsStatus,
    EventsWatchOptions,
};
use parking_lot::Mutex;

use crate::FakeClockPort;
use crate::script::{CallLog, Script, StreamGauge, Timeline};
use crate::scripted_feed::TICK;

/// Builds a domain [`Event`]; start with [`event`].
#[derive(Debug, Clone)]
pub struct EventBuilder(Event);

/// A `Normal` event `Scheduled` about pod `default/web-0` of cluster `kubeconfig:kind-oxikube`,
/// seen once at `2026-01-01T00:00:00Z`. Change what a test cares about.
pub fn event() -> EventBuilder {
    let cluster = ClusterId::new("kubeconfig", &ContextName::new("kind-oxikube"));
    let at: Timestamp = "2026-01-01T00:00:00Z"
        .parse()
        .unwrap_or(Timestamp::UNIX_EPOCH);
    EventBuilder(Event {
        uid: Some(Arc::from("event-uid")),
        namespace: Some(Arc::from("default")),
        event_type: EventType::Normal,
        reason: Arc::from("Scheduled"),
        message: "Successfully assigned default/web-0 to node-1".into(),
        truncated: false,
        regarding: ResourceRef::new(
            cluster,
            Gvk::new("", "v1", "Pod"),
            Some("default".into()),
            "web-0",
        ),
        regarding_uid: Some(Arc::from("pod-uid")),
        related: None,
        count: 1,
        first_seen: Some(at),
        last_seen: Some(at),
        reporting_component: Some(Arc::from("default-scheduler")),
        reporting_instance: None,
        action: None,
    })
}

impl EventBuilder {
    /// The event's own uid.
    #[must_use]
    pub fn uid(mut self, uid: &str) -> Self {
        self.0.uid = Some(Arc::from(uid));
        self
    }

    /// Type `Warning`.
    #[must_use]
    pub fn warning(mut self) -> Self {
        self.0.event_type = EventType::Warning;
        self
    }

    /// The reason.
    #[must_use]
    pub fn reason(mut self, reason: &str) -> Self {
        self.0.reason = Arc::from(reason);
        self
    }

    /// The message.
    #[must_use]
    pub fn message(mut self, message: &str) -> Self {
        self.0.message = message.to_owned();
        self
    }

    /// The object the event is about (`kind` in the core group) and its uid.
    #[must_use]
    pub fn regarding(mut self, kind: &str, namespace: Option<&str>, name: &str, uid: &str) -> Self {
        let cluster = self.0.regarding.cluster.clone();
        self.0.regarding = ResourceRef::new(
            cluster,
            Gvk::new("", "v1", kind),
            namespace.map(Arc::from),
            name,
        );
        self.0.regarding_uid = Some(Arc::from(uid));
        self
    }

    /// How many times it happened.
    #[must_use]
    pub fn count(mut self, count: u32) -> Self {
        self.0.count = count;
        self
    }

    /// When it was last seen (RFC 3339).
    ///
    /// # Panics
    ///
    /// When `at` is not an RFC 3339 timestamp.
    #[must_use]
    pub fn last_seen(mut self, at: &str) -> Self {
        self.0.last_seen = Some(at.parse().expect("an RFC 3339 timestamp"));
        self
    }

    /// The event.
    pub fn build(self) -> Event {
        self.0
    }
}

/// A scripted events feed: the initial list at tick 0 and the changes of each later tick.
/// See the [module docs](self).
#[derive(Debug, Clone, Default)]
pub struct FakeEventFeed {
    initial: Vec<Event>,
    steps: BTreeMap<u32, Vec<Delta<Event>>>,
}

impl FakeEventFeed {
    /// An empty feed.
    pub fn new() -> Self {
        Self::default()
    }

    /// The events listed first (tick 0, one `Restarted` batch).
    #[must_use]
    pub fn initial(mut self, events: impl IntoIterator<Item = Event>) -> Self {
        self.initial = events.into_iter().collect();
        self
    }

    /// `event` is added or changed at `tick` (`tick >= 1`).
    #[must_use]
    pub fn add(self, tick: u32, event: Event) -> Self {
        self.at(tick, [Delta::Applied(event)])
    }

    /// `event` is deleted (or evicted) at `tick`.
    #[must_use]
    pub fn delete(self, tick: u32, event: Event) -> Self {
        self.at(tick, [Delta::Deleted(event)])
    }

    /// `deltas` arrive at `tick`, in one batch and in this order.
    ///
    /// # Panics
    ///
    /// When `tick` is 0: the initial list is [`initial`](Self::initial).
    #[must_use]
    pub fn at(mut self, tick: u32, deltas: impl IntoIterator<Item = Delta<Event>>) -> Self {
        assert!(tick >= 1, "tick 0 is the initial list; use `initial`");
        self.steps.entry(tick).or_default().extend(deltas);
        self
    }

    /// The last tick that delivers something (0 when only the initial list is scripted).
    pub fn last_tick(&self) -> u32 {
        self.steps.keys().next_back().copied().unwrap_or(0)
    }

    /// Every batch with its tick, oldest first.
    pub fn batches(&self) -> Vec<(u32, DeltaBatch<Event>)> {
        let first = DeltaBatch::from_deltas(vec![Delta::Restarted(self.initial.clone())]);
        std::iter::once((0, first))
            .chain(
                self.steps
                    .iter()
                    .map(|(t, d)| (*t, DeltaBatch::from_deltas(d.clone()))),
            )
            .collect()
    }

    /// The feed as a timeline: tick `n` at offset `n * TICK`, kept open like a live watch.
    pub fn timeline(&self) -> Timeline<DeltaBatch<Event>> {
        self.batches()
            .into_iter()
            .fold(Timeline::new(), |timeline, (tick, batch)| {
                timeline.ok_at(TICK * tick, batch)
            })
            .keep_open()
    }
}

/// A [`EventsMonitor`] whose [`EventsStatus`] the test sets.
#[derive(Debug, Default)]
pub struct FakeEventsMonitor(Mutex<EventsStatus>);

impl FakeEventsMonitor {
    /// Replaces the status.
    pub fn set(&self, status: EventsStatus) {
        *self.0.lock() = status;
    }
}

impl EventsMonitor for FakeEventsMonitor {
    fn status(&self) -> EventsStatus {
        self.0.lock().clone()
    }
}

/// Queued responses for each [`FakeEventsPort`] method.
#[derive(Debug, Default)]
pub struct EventsScripts {
    /// `watch_events`: the feed to serve (or the error to fail with), one per call.
    pub watch_events: Script<FakeEventFeed>,
}

/// One call made on a [`FakeEventsPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventsCall {
    /// `watch_events(scope, options)`.
    Watch(WatchScope, EventsWatchOptions),
}

/// Fake `EventsPort`.
///
/// `watch_events` serves the next scripted [`FakeEventFeed`], replayed on [`clock`](Self::clock)
/// (move it by [`TICK`] per tick), or, with nothing scripted, an empty feed that stays open.
/// The feed's [`EventsStatus`] comes from [`monitor`](Self::monitor).
pub struct FakeEventsPort {
    script: EventsScripts,
    calls: CallLog<EventsCall>,
    clock: Arc<FakeClockPort>,
    monitor: Arc<FakeEventsMonitor>,
    feeds: StreamGauge,
}

fake_plumbing!(FakeEventsPort, EventsScripts, EventsCall);

impl Default for FakeEventsPort {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for FakeEventsPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeEventsPort")
            .field("calls", &self.calls.len())
            .finish_non_exhaustive()
    }
}

impl FakeEventsPort {
    /// A fake with its own clock and a status reporting the core API, nothing forbidden.
    pub fn new() -> Self {
        let monitor = FakeEventsMonitor::default();
        monitor.set(EventsStatus {
            apis: vec![EventApi::EventsV1],
            ..EventsStatus::default()
        });
        Self {
            script: EventsScripts::default(),
            calls: CallLog::default(),
            clock: Arc::new(FakeClockPort::default()),
            monitor: Arc::new(monitor),
            feeds: StreamGauge::default(),
        }
    }

    /// Queues `feed` as the next feed served.
    pub fn install(&self, feed: FakeEventFeed) {
        self.script.watch_events.push_ok(feed);
    }

    /// The clock feeds are replayed on; advance it to deliver scripted ticks.
    pub fn clock(&self) -> &Arc<FakeClockPort> {
        &self.clock
    }

    /// The status every served feed reports.
    pub fn monitor(&self) -> &Arc<FakeEventsMonitor> {
        &self.monitor
    }

    /// Feeds served and not yet dropped: proves a consumer opened one feed per cluster and
    /// stopped it.
    pub fn live_feeds(&self) -> usize {
        self.feeds.live()
    }
}

#[async_trait]
impl EventsPort for FakeEventsPort {
    async fn watch_events(
        &self,
        scope: &WatchScope,
        options: &EventsWatchOptions,
    ) -> OxiResult<EventsFeed> {
        self.calls
            .record(EventsCall::Watch(scope.clone(), options.clone()));
        let feed = self
            .script
            .watch_events
            .next_or_else(|| Ok(FakeEventFeed::new()))?;
        let clock: Arc<dyn ClockPort> = self.clock.clone();
        Ok(EventsFeed {
            feed: self.feeds.track(feed.timeline().replay(clock)),
            monitor: self.monitor.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt as _;
    use futures::executor::block_on;

    use super::*;

    #[test]
    fn ticks_replay_on_the_clock_and_a_dropped_feed_is_not_live() {
        let port = FakeEventsPort::new();
        port.install(
            FakeEventFeed::new()
                .initial([event().uid("a").build()])
                .add(1, event().uid("b").warning().build())
                .delete(2, event().uid("a").build()),
        );
        let mut served =
            block_on(port.watch_events(&WatchScope::Cluster, &EventsWatchOptions::default()))
                .unwrap();
        assert_eq!(port.live_feeds(), 1);
        let first = block_on(served.feed.next()).unwrap().unwrap();
        assert!(matches!(first.deltas.as_slice(), [Delta::Restarted(all)] if all.len() == 1));
        port.clock().advance(TICK);
        let second = block_on(served.feed.next()).unwrap().unwrap();
        assert!(matches!(second.deltas.as_slice(), [Delta::Applied(e)] if e.is_warning()));
        port.clock().advance(TICK);
        let third = block_on(served.feed.next()).unwrap().unwrap();
        assert!(matches!(third.deltas.as_slice(), [Delta::Deleted(_)]));
        assert_eq!(served.monitor.status().apis, [EventApi::EventsV1]);
        drop(served);
        assert_eq!(port.live_feeds(), 0);
        assert_eq!(port.recorded_calls().len(), 1);
    }

    #[test]
    #[should_panic(expected = "tick 0")]
    fn tick_zero_is_the_initial_list() {
        let _ = FakeEventFeed::new().add(0, event().build());
    }
}
