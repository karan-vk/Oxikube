//! The service against fake sessions and `FakeEventsPort`: one feed per cluster whatever the
//! subscribers, coalesced changes, the idle grace, disconnects and reconnects, the bound from
//! the config, final failures and retry. The feed tasks run on the Tokio test runtime; the
//! scripted feed replays on the port's clock and the grace and retry pauses on the service's,
//! so every wait is on a condition with a deadline, never a sleep before an assertion.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::OxiError;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::{NamespaceSelection, WatchScope};
use oxikube_ports::{ClusterContext, EventsWatchOptions, SourceId};
use oxikube_testkit::{
    EventsCall, FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeEventFeed,
    FakeEventsPort, TICK,
};

use super::{ev, t0, uids, warning};
use crate::events::{EventFilter, EventService, EventsChanged, EventsConfig, EventsState};
use crate::overview::RecentWarnings;
use crate::session::ClusterSessionManager;
use crate::store::Spawner;

const GRACE: Duration = Duration::from_secs(30);

fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("a"))
}

struct Harness {
    sessions: ClusterSessionManager,
    connector: Arc<FakeClusterConnectorPort>,
    clock: Arc<FakeClockPort>,
    service: EventService,
}

impl Harness {
    fn new() -> Self {
        Self::with(EventsConfig {
            buffer: 500,
            idle_grace: GRACE,
            ..EventsConfig::default()
        })
    }

    fn with(config: EventsConfig) -> Self {
        let entry = ClusterContext::new(
            cluster(),
            ContextName::new("a"),
            SourceId("kubeconfig".into()),
        );
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
        let sessions = ClusterSessionManager::new(
            connector.clone(),
            source,
            Arc::new(FakeClockPort::default()),
        );
        let clock = Arc::new(FakeClockPort::new(t0()));
        let service = EventService::new(sessions.clone(), clock.clone(), config);
        let spawner: Arc<dyn Spawner> = Arc::new(|task| {
            tokio::spawn(task);
        });
        service.start(spawner);
        Self {
            sessions,
            connector,
            clock,
            service,
        }
    }

    fn port(&self) -> Arc<FakeEventsPort> {
        self.connector.ports_for(&cluster()).events
    }

    async fn connect(&self) {
        self.sessions.connect(&cluster()).await.expect("connect");
    }

    async fn until(&self, what: &str, done: impl Fn(&Self) -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !done(self) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for: {what}"));
    }

    async fn ready(&self) {
        self.until("the feed is ready", |h| {
            h.service.state(&cluster()) == EventsState::Ready
        })
        .await;
    }

    fn all(&self) -> Vec<String> {
        uids(&self.service.query(&cluster(), &EventFilter::new()))
    }
}

/// The next item of `sub`, within the deadline.
async fn next(sub: &mut crate::events::EventsSubscription) -> EventsChanged {
    tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("an item in time")
        .expect("the stream does not end")
}

#[tokio::test]
async fn subscribers_share_one_cluster_wide_feed_bounded_by_the_config() {
    let h = Harness::new();
    h.connect().await;
    h.port()
        .install(FakeEventFeed::new().initial([ev("a", "ns", "p", 1)]));
    let _one = h.service.subscribe(&cluster(), EventFilter::new());
    let _two = h.service.subscribe(&cluster(), EventFilter::warnings());
    h.ready().await;
    assert_eq!(h.port().live_feeds(), 1);
    assert_eq!(
        h.port().recorded_calls(),
        [EventsCall::Watch(
            WatchScope::Cluster,
            EventsWatchOptions::default().capacity(500)
        )]
    );
    assert_eq!(h.all(), ["a"]);
    assert!(h.service.is_watching(&cluster()));
}

#[tokio::test]
async fn a_subscriber_gets_a_reset_then_coalesced_changes_through_its_filter() {
    let h = Harness::new();
    h.connect().await;
    h.port().install(
        FakeEventFeed::new()
            .initial([ev("a", "ns", "p", 1)])
            .add(1, warning("w", "ns", "p", 2, "BackOff"))
            .add(2, ev("n", "ns", "p", 3))
            .add(3, {
                let mut w = warning("w", "ns", "p", 4, "BackOff");
                w.count = 5;
                w
            }),
    );
    let mut sub = h.service.subscribe(&cluster(), EventFilter::warnings());
    assert!(next(&mut sub).await.reset);
    h.ready().await;
    // The initial list replaced the (empty) buffer: read it again.
    let listed = next(&mut sub).await;
    assert!(listed.reset);
    assert_eq!(listed.state, Some(EventsState::Ready));
    // Three ticks land before the subscriber looks again: one item.
    for tick in 1..=3 {
        h.port().clock().advance(TICK);
        h.until("the tick applied", |h| h.all().len() == 1 + tick.min(2))
            .await;
    }
    h.until("the update applied", |h| {
        h.service.latest(&cluster(), &EventFilter::warnings(), 1)[0].count == 5
    })
    .await;
    let item = next(&mut sub).await;
    assert!(!item.reset);
    assert_eq!(uids(&item.added), ["w"], "the Normal event is filtered out");
    assert_eq!(
        item.added[0].count, 5,
        "added then updated: one addition, newest state"
    );
    assert!(item.updated.is_empty());
    assert_eq!(uids(&sub.query()), ["w"]);
    assert_eq!(h.all(), ["w", "n", "a"], "deduped by uid, newest first");
}

#[tokio::test]
async fn the_feed_stops_after_the_grace_once_the_last_subscriber_leaves() {
    let h = Harness::new();
    h.connect().await;
    let sub = h.service.subscribe(&cluster(), EventFilter::new());
    h.ready().await;
    drop(sub);
    h.until("the grace timer sleeps", |h| {
        h.clock.pending_sleepers() == 1
    })
    .await;
    h.clock.advance(GRACE / 2);
    // Back within the grace: the same feed, and the timer that fires later is void.
    let sub = h.service.subscribe(&cluster(), EventFilter::new());
    h.clock.advance(GRACE);
    h.until("the void timer fired", |h| h.clock.pending_sleepers() == 0)
        .await;
    assert_eq!(h.port().live_feeds(), 1);
    assert_eq!(h.port().recorded_calls().len(), 1);
    drop(sub);
    h.until("the grace timer sleeps", |h| {
        h.clock.pending_sleepers() == 1
    })
    .await;
    h.clock.advance(GRACE);
    h.until("the feed is dropped", |h| h.port().live_feeds() == 0)
        .await;
    assert_eq!(h.service.state(&cluster()), EventsState::Idle);
    assert!(h.all().is_empty(), "the events are forgotten");
}

#[tokio::test]
async fn nothing_watches_until_connected_and_a_disconnect_stops_and_forgets() {
    let h = Harness::new();
    h.port()
        .install(FakeEventFeed::new().initial([ev("a", "ns", "p", 1)]));
    let mut sub = h.service.subscribe(&cluster(), EventFilter::new());
    assert!(next(&mut sub).await.reset);
    tokio::task::yield_now().await;
    assert!(h.port().recorded_calls().is_empty());
    assert_eq!(h.service.state(&cluster()), EventsState::Idle);

    h.connect().await;
    h.ready().await;
    assert_eq!(h.all(), ["a"]);

    h.sessions.disconnect(&cluster()).expect("disconnect");
    h.until("the feed is dropped", |h| h.port().live_feeds() == 0)
        .await;
    assert!(h.all().is_empty());
    assert_eq!(h.service.state(&cluster()), EventsState::Idle);

    h.connect().await;
    h.ready().await;
    assert_eq!(h.port().recorded_calls().len(), 2, "reopened on reconnect");
    drop(sub);
}

#[tokio::test]
async fn a_new_bound_trims_at_once_and_reopens_the_feed_with_it() {
    let h = Harness::new();
    h.connect().await;
    let events: Vec<_> = (0..200)
        .map(|i| ev(&format!("e{i}"), "ns", "p", i))
        .collect();
    h.port()
        .install(FakeEventFeed::new().initial(events.clone()));
    h.port().install(FakeEventFeed::new().initial(events));
    let _sub = h.service.subscribe(&cluster(), EventFilter::new());
    h.ready().await;
    assert_eq!(h.all().len(), 200);

    let mut config = h.service.config();
    config.cluster_buffers.insert(cluster(), 150);
    h.service.set_config(config);
    h.until("the relist", |h| h.port().recorded_calls().len() == 2)
        .await;
    h.until("the new bound", |h| h.all().len() == 150).await;
    assert_eq!(h.all()[0], "e199", "the newest stay");
    let EventsCall::Watch(_, options) = h.port().recorded_calls()[1].clone();
    assert_eq!(options.capacity, Some(150));
}

#[tokio::test]
async fn a_final_failure_is_reported_until_retry() {
    let h = Harness::new();
    h.connect().await;
    h.port()
        .script()
        .watch_events
        .push_err(OxiError::forbidden("events are forbidden"));
    h.port()
        .install(FakeEventFeed::new().initial([ev("a", "ns", "p", 1)]));
    let mut sub = h.service.subscribe(&cluster(), EventFilter::new());
    assert!(next(&mut sub).await.reset);
    h.until("failed", |h| h.service.state(&cluster()).is_failed())
        .await;
    let item = next(&mut sub).await;
    assert!(matches!(item.state, Some(EventsState::Failed { .. })));
    h.service.retry(&cluster());
    h.ready().await;
    assert_eq!(h.all(), ["a"]);
}

#[tokio::test]
async fn a_failed_feed_follows_a_new_namespace_selection() {
    let h = Harness::new();
    h.connect().await;
    h.port()
        .script()
        .watch_events
        .push_err(OxiError::forbidden("cluster-wide events are forbidden"));
    let _sub = h.service.subscribe(&cluster(), EventFilter::new());
    h.until("failed", |h| h.service.state(&cluster()).is_failed())
        .await;
    h.sessions
        .set_namespace_selection(&cluster(), NamespaceSelection::single("team"))
        .expect("select");
    h.ready().await;
    let EventsCall::Watch(_, options) = h.port().recorded_calls()[1].clone();
    assert_eq!(options.fallback_namespaces, ["team"]);
}

#[tokio::test]
async fn a_retryable_open_failure_is_retried_after_a_pause() {
    let h = Harness::new();
    h.connect().await;
    h.port()
        .script()
        .watch_events
        .push_err(OxiError::network("connection reset"));
    let _sub = h.service.subscribe(&cluster(), EventFilter::new());
    h.until("retrying", |h| {
        matches!(h.service.state(&cluster()), EventsState::Retrying { .. })
    })
    .await;
    h.until("the retry pause", |h| h.clock.pending_sleepers() > 0)
        .await;
    h.clock.advance(Duration::from_secs(1));
    h.ready().await;
    assert_eq!(h.port().recorded_calls().len(), 2);
}

#[tokio::test]
async fn the_overview_reads_the_newest_warnings() {
    let h = Harness::new();
    h.connect().await;
    let mut old = warning("w1", "ns", "pod-a", 1, "BackOff");
    old.count = 4;
    h.port().install(FakeEventFeed::new().initial([
        old,
        ev("n", "ns", "pod-a", 5),
        warning("w2", "ns", "pod-b", 3, "FailedScheduling"),
    ]));
    let _sub = h.service.subscribe(&cluster(), EventFilter::new());
    h.ready().await;
    let rows = RecentWarnings::latest(&h.service, &cluster(), 5);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].object, "Pod/pod-b");
    assert_eq!(rows[0].reason, "FailedScheduling");
    assert_eq!(rows[1].count, 4);
    assert_eq!(RecentWarnings::latest(&h.service, &cluster(), 1).len(), 1);
    assert!(
        RecentWarnings::latest(&h.service, &ClusterId::new("x", &ContextName::new("y")), 5)
            .is_empty()
    );
}
