//! Badges and the per-object feed against `FakeEventsPort` and fake sessions: only warnings
//! badge, the newest warning of an object shows, a badge expires past the window without any
//! event, a removed event gives way to an older one, and the Events tab's feed lists an object's
//! events newest first.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use jiff::{SignedDuration, Timestamp};
use oxikube_domain::event::Event;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeEventFeed, FakeEventsPort,
    TICK, event,
};

use super::{DEFAULT_WARNING_WINDOW, WarningBadges};
use crate::events::{EventService, EventsConfig, EventsState};
use crate::overview::RecentWarnings;
use crate::session::ClusterSessionManager;
use crate::store::Spawner;

fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("a"))
}

fn t0() -> Timestamp {
    "2026-01-01T00:00:00Z".parse().expect("a timestamp")
}

fn at(secs: i64) -> Timestamp {
    t0() + SignedDuration::from_secs(secs)
}

/// A `Normal` event `uid` about the pod `object` (uid `object`) last seen `secs` after [`t0`].
fn normal(uid: &str, object: &str, secs: i64) -> Event {
    let mut e = event()
        .uid(uid)
        .reason("Pulled")
        .regarding("Pod", Some("ns"), object, object)
        .build();
    e.last_seen = Some(at(secs));
    e
}

/// [`normal`] as a `Warning` with `reason`.
fn warning(uid: &str, object: &str, secs: i64, reason: &str) -> Event {
    let mut e = normal(uid, object, secs);
    e.event_type = oxikube_domain::event::EventType::Warning;
    e.reason = Arc::from(reason);
    e
}

struct Harness {
    sessions: ClusterSessionManager,
    connector: Arc<FakeClusterConnectorPort>,
    service: EventService,
    badges: WarningBadges,
}

impl Harness {
    fn new() -> Self {
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
        let service = EventService::new(
            sessions.clone(),
            clock.clone(),
            EventsConfig {
                buffer: 500,
                ..EventsConfig::default()
            },
        );
        let spawner: Arc<dyn Spawner> = Arc::new(|task| {
            tokio::spawn(task);
        });
        service.start(spawner.clone());
        let badges = WarningBadges::new(service.clone(), clock, spawner, DEFAULT_WARNING_WINDOW);
        Self {
            sessions,
            connector,
            service,
            badges,
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

    fn reason_of(&self, uid: &str, now: Timestamp) -> Option<String> {
        self.badges
            .for_cluster(&cluster())
            .get(uid, now)
            .map(|b| b.reason().to_owned())
    }
}

#[tokio::test]
async fn only_warnings_badge_an_object() {
    let h = Harness::new();
    h.connect().await;
    h.port().install(FakeEventFeed::new().initial([
        normal("e1", "calm", 10),
        warning("e2", "sick", 20, "BackOff"),
    ]));
    let _track = h.badges.track(&cluster());
    h.until("the warning is badged", |h| {
        h.reason_of("sick", at(30)).is_some()
    })
    .await;
    assert_eq!(h.reason_of("sick", at(30)).as_deref(), Some("BackOff"));
    assert_eq!(
        h.reason_of("calm", at(30)),
        None,
        "a Normal event is no badge"
    );
    assert_eq!(h.badges.for_cluster(&cluster()).len(), 1);
}

#[tokio::test]
async fn a_new_warning_badges_the_object_and_wakes_the_views_once() {
    let h = Harness::new();
    h.connect().await;
    let port = h.port();
    port.install(
        FakeEventFeed::new()
            .initial([normal("e1", "web", 5)])
            .add(1, warning("w1", "web", 60, "Unhealthy"))
            .add(1, warning("w2", "db", 61, "BackOff")),
    );
    let mut changes = h.badges.changes();
    let _track = h.badges.track(&cluster());
    h.until("the feed is ready", |h| {
        h.service.state(&cluster()) == EventsState::Ready
    })
    .await;
    // The first report (the reset) wakes the views.
    tokio::time::timeout(Duration::from_secs(5), changes.next())
        .await
        .expect("woken")
        .expect("a change");
    assert_eq!(h.reason_of("web", at(70)), None);

    port.clock().advance(TICK);
    h.until("both warnings are badged", |h| {
        h.reason_of("web", at(70)).is_some() && h.reason_of("db", at(70)).is_some()
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), changes.next())
        .await
        .expect("woken")
        .expect("a change");
}

#[tokio::test]
async fn the_newest_warning_of_an_object_is_the_badge() {
    let h = Harness::new();
    h.connect().await;
    h.port().install(
        FakeEventFeed::new()
            .initial([warning("old", "web", 10, "FailedMount")])
            .add(1, warning("new", "web", 90, "BackOff"))
            // Delivered late but older than what shows: the badge stays.
            .add(1, warning("older", "web", 20, "Failed")),
    );
    let _track = h.badges.track(&cluster());
    h.until("the first warning shows", |h| {
        h.reason_of("web", at(100)).as_deref() == Some("FailedMount")
    })
    .await;
    h.port().clock().advance(TICK);
    h.until("the newer warning shows", |h| {
        h.reason_of("web", at(100)).as_deref() == Some("BackOff")
    })
    .await;
    let badge = h
        .badges
        .for_cluster(&cluster())
        .get("web", at(100))
        .unwrap();
    assert_eq!(badge.last_seen(), at(90));
    assert_eq!(badge.count(), 1);
}

#[tokio::test]
async fn a_badge_expires_past_the_window_without_any_event() {
    let h = Harness::new();
    h.connect().await;
    h.port()
        .install(FakeEventFeed::new().initial([warning("w", "web", 100, "BackOff")]));
    let _track = h.badges.track(&cluster());
    h.until("badged", |h| h.reason_of("web", at(101)).is_some())
        .await;
    let badges = h.badges.for_cluster(&cluster());
    let hour = DEFAULT_WARNING_WINDOW.as_secs() as i64;
    assert!(badges.get("web", at(100 + hour)).is_some(), "still within");
    assert_eq!(badges.expired(at(100 + hour)), 0);
    assert!(badges.get("web", at(101 + hour)).is_none(), "an hour on");
    assert_eq!(badges.expired(at(101 + hour)), 1);

    // A longer window brings it back at once, and wakes the views.
    let mut changes = h.badges.changes();
    h.badges.set_window(Duration::from_secs(2 * 3_600));
    assert!(badges.get("web", at(101 + hour)).is_some());
    assert_eq!(badges.expired(at(101 + hour)), 0);
    tokio::time::timeout(Duration::from_secs(5), changes.next())
        .await
        .expect("woken by the window")
        .expect("a change");
}

#[tokio::test]
async fn the_window_is_clamped() {
    let h = Harness::new();
    h.badges.set_window(Duration::from_secs(1));
    assert_eq!(h.badges.window(), super::MIN_WARNING_WINDOW);
    h.badges.set_window(Duration::from_secs(u64::MAX / 2));
    assert_eq!(h.badges.window(), super::MAX_WARNING_WINDOW);
}

#[tokio::test]
async fn a_deleted_warning_removes_the_badge_or_gives_way_to_an_older_one() {
    let h = Harness::new();
    h.connect().await;
    let newest = warning("new", "web", 90, "BackOff");
    let only = warning("only", "db", 50, "Failed");
    h.port().install(
        FakeEventFeed::new()
            .initial([
                warning("old", "web", 10, "FailedMount"),
                newest.clone(),
                only.clone(),
            ])
            .delete(1, newest)
            .delete(1, only),
    );
    let _track = h.badges.track(&cluster());
    h.until("badged", |h| {
        h.reason_of("web", at(100)).as_deref() == Some("BackOff")
            && h.reason_of("db", at(100)).is_some()
    })
    .await;
    h.port().clock().advance(TICK);
    h.until("the older warning shows again", |h| {
        h.reason_of("web", at(100)).as_deref() == Some("FailedMount")
    })
    .await;
    assert_eq!(h.reason_of("db", at(100)), None, "its only warning is gone");
}

#[tokio::test]
async fn a_disconnect_forgets_the_badges() {
    let h = Harness::new();
    h.connect().await;
    h.port()
        .install(FakeEventFeed::new().initial([warning("w", "web", 10, "BackOff")]));
    let _track = h.badges.track(&cluster());
    h.until("badged", |h| h.reason_of("web", at(20)).is_some())
        .await;
    h.sessions.disconnect(&cluster()).expect("disconnect");
    h.until("forgotten", |h| h.reason_of("web", at(20)).is_none())
        .await;
}

#[tokio::test]
async fn a_cluster_scoped_object_is_also_badged_under_its_name() {
    let h = Harness::new();
    h.connect().await;
    // The kubelet records a Node's events with the node's name as the uid.
    let mut node = warning("n1", "node-1", 10, "NodeNotReady");
    node.regarding.gvk = oxikube_domain::ids::Gvk::new("", "v1", "Node");
    node.regarding_uid = Some(Arc::from("node-1"));
    h.port().install(FakeEventFeed::new().initial([node]));
    let _track = h.badges.track(&cluster());
    h.until("badged", |h| h.reason_of("node-1", at(20)).is_some())
        .await;
    let badges = h.badges.for_cluster(&cluster());
    assert!(badges.get_for(Some("real-uid"), None, at(20)).is_none());
    assert!(
        badges
            .get_for(Some("real-uid"), Some("node-1"), at(20))
            .is_some()
    );
}

#[tokio::test]
async fn the_overview_panel_lists_the_warnings_within_the_window() {
    let h = Harness::new();
    h.connect().await;
    // The fake clock stands at `t0 + 2h`: the first warning is two hours old.
    h.port().install(FakeEventFeed::new().initial([
        warning("stale", "old-pod", 0, "BackOff"),
        warning("fresh", "new-pod", 2 * 3_600 - 60, "Failed"),
        normal("n", "other", 2 * 3_600),
    ]));
    let _track = h.badges.track(&cluster());
    h.until("both held", |h| {
        h.service
            .latest(&cluster(), &crate::events::EventFilter::warnings(), 10)
            .len()
            == 2
    })
    .await;
    // Move the badges' clock two hours on.
    let h2 = WarningBadges::new(
        h.service.clone(),
        Arc::new(FakeClockPort::new(at(2 * 3_600))),
        Arc::new(|task| {
            tokio::spawn(task);
        }),
        DEFAULT_WARNING_WINDOW,
    );
    let rows = RecentWarnings::latest(&h2, &cluster(), 10);
    assert_eq!(
        rows.iter().map(|r| r.object.as_str()).collect::<Vec<_>>(),
        ["Pod/new-pod"],
        "the stale warning is past the window"
    );
    assert_eq!(RecentWarnings::latest(&h.service, &cluster(), 10).len(), 2);
}

#[tokio::test]
async fn an_objects_feed_lists_its_events_newest_first() {
    let h = Harness::new();
    h.connect().await;
    let port = h.port();
    port.install(
        FakeEventFeed::new()
            .initial([
                normal("e1", "web", 10),
                warning("e2", "web", 30, "BackOff"),
                normal("e3", "other", 40),
                normal("e4", "web", 20),
            ])
            .add(1, warning("e5", "web", 50, "Failed")),
    );
    let mut feed = h.badges.object_feed(&cluster(), Some("web"), None);
    tokio::time::timeout(Duration::from_secs(5), feed.next())
        .await
        .expect("a first item")
        .expect("not ended");
    let reasons = |feed: &super::ObjectEventsFeed| {
        feed.events()
            .iter()
            .map(|e| e.uid.as_deref().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    // Items may arrive in pieces; wait for the listing.
    while reasons(&feed).len() < 3 {
        tokio::time::timeout(Duration::from_secs(5), feed.next())
            .await
            .expect("a change")
            .expect("not ended");
    }
    assert_eq!(reasons(&feed), ["e2", "e4", "e1"]);

    port.clock().advance(TICK);
    while reasons(&feed).len() < 4 {
        tokio::time::timeout(Duration::from_secs(5), feed.next())
            .await
            .expect("a change")
            .expect("not ended");
    }
    assert_eq!(reasons(&feed), ["e5", "e2", "e4", "e1"]);
}

#[tokio::test]
async fn a_nodes_feed_merges_its_uid_and_its_name() {
    let h = Harness::new();
    h.connect().await;
    let on = |uid: &str, secs, regarding_uid: &str| {
        let mut e = normal(uid, "node-1", secs);
        e.regarding.gvk = oxikube_domain::ids::Gvk::new("", "v1", "Node");
        e.regarding_uid = Some(Arc::from(regarding_uid));
        e
    };
    h.port().install(
        FakeEventFeed::new().initial([on("by-uid", 10, "node-uid"), on("by-name", 20, "node-1")]),
    );
    let mut feed = h
        .badges
        .object_feed(&cluster(), Some("node-uid"), Some("node-1"));
    while feed.events().len() < 2 {
        tokio::time::timeout(Duration::from_secs(5), feed.next())
            .await
            .expect("a change")
            .expect("not ended");
    }
    let uids: Vec<_> = feed
        .events()
        .iter()
        .map(|e| e.uid.clone().unwrap())
        .collect();
    assert_eq!(
        uids.iter().map(|u| &**u).collect::<Vec<_>>(),
        ["by-name", "by-uid"]
    );
}
