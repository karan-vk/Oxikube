//! The service against fake sessions, `FakeEventsPort` and `FakeStatePort`: Warning events of a
//! connected cluster become notifications (and the backlog does not), a muted cluster is not
//! watched and stays muted after a restart, subscribers get coalesced items, and the raised
//! notifications reach the OS notifier once opted in. Feed tasks run on the Tokio test runtime;
//! every wait polls a condition with a deadline.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::ids::ContextName;
use oxikube_ports::{ClusterContext, SourceId, StateKey, StatePort};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeEventFeed, FakeEventsPort,
    FakeNotifierPort, FakeStatePort, TICK, event,
};

use super::{cluster, input, t0, warning};
use crate::events::{EventService, EventsConfig};
use crate::notifications::{
    ClusterRules, MUTED_KEY, NotificationRules, NotificationService, NotificationsChanged,
    forward_to_os,
};
use crate::os_notifications::OsNotificationService;
use crate::session::ClusterSessionManager;
use crate::store::Spawner;

struct Harness {
    sessions: ClusterSessionManager,
    connector: Arc<FakeClusterConnectorPort>,
    state: Arc<FakeStatePort>,
    events: EventService,
    centre: NotificationService,
}

fn spawner() -> Arc<dyn Spawner> {
    Arc::new(|task| {
        tokio::spawn(task);
    })
}

impl Harness {
    fn new() -> Self {
        Self::with_state(Arc::new(FakeStatePort::new()))
    }

    fn with_state(state: Arc<FakeStatePort>) -> Self {
        let entry = ClusterContext::new(
            cluster(),
            ContextName::new("kind-oxikube"),
            SourceId("kubeconfig".into()),
        );
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
        let sessions = ClusterSessionManager::new(
            connector.clone(),
            source,
            Arc::new(FakeClockPort::default()),
        );
        let events = EventService::new(
            sessions.clone(),
            Arc::new(FakeClockPort::new(t0())),
            EventsConfig {
                buffer: 2_000,
                idle_grace: Duration::from_secs(30),
                ..EventsConfig::default()
            },
        );
        events.start(spawner());
        let centre = NotificationService::new(
            events.clone(),
            sessions.clone(),
            state.clone(),
            Arc::new(FakeClockPort::new(t0())),
        );
        centre.start(spawner());
        Self {
            sessions,
            connector,
            state,
            events,
            centre,
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

    /// Delivers the feed's next tick.
    async fn tick(&self) {
        self.port().clock().advance(TICK);
        tokio::task::yield_now().await;
    }

    fn titles(&self) -> Vec<String> {
        self.centre.list().iter().map(|n| n.title.clone()).collect()
    }
}

async fn next(sub: &mut crate::notifications::NotificationsSubscription) -> NotificationsChanged {
    tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("an item in time")
        .expect("the stream does not end")
}

#[tokio::test]
async fn a_new_warning_on_a_watched_kind_is_listed_and_raised_and_the_backlog_is_not() {
    let h = Harness::new();
    h.port().install(
        FakeEventFeed::new()
            .initial([warning("old", "web-0", "BackOff", 7, -3_600)])
            .add(1, warning("new", "web-1", "BackOff", 1, 1))
            .add(2, {
                let mut normal = warning("normal", "web-2", "Pulled", 1, 2);
                normal.event_type = oxikube_domain::event::EventType::Normal;
                normal
            })
            .add(3, {
                let mut cm = event()
                    .uid("cm")
                    .warning()
                    .reason("Invalid")
                    .regarding("ConfigMap", Some("default"), "cfg", "cfg")
                    .build();
                cm.last_seen = Some(super::at(3));
                cm
            }),
    );
    let mut sub = h.centre.subscribe();
    assert!(
        next(&mut sub).await.changed,
        "the first item says read the list"
    );
    h.connect().await;
    h.until("the cluster is watched", |h| {
        h.centre.is_watching(&cluster())
    })
    .await;
    h.until("the feed is listed", |h| {
        h.events.is_watching(&cluster()) && h.port().live_feeds() == 1
    })
    .await;
    for _ in 0..3 {
        h.tick().await;
    }
    h.until("the new warning is listed", |h| !h.centre.list().is_empty())
        .await;
    let item = next(&mut sub).await;
    assert!(item.changed);
    assert_eq!(item.raised.len(), 1);
    assert_eq!(item.raised[0].title, "Pod default/web-1: BackOff");
    assert_eq!(
        h.titles(),
        ["Pod default/web-1: BackOff"],
        "no backlog, no Normal, no ConfigMap"
    );
    assert_eq!(h.centre.unread(), 1);
    assert_eq!(
        h.port().live_feeds(),
        1,
        "the centre shares the cluster's one feed"
    );
}

#[tokio::test]
async fn a_rising_count_updates_the_row_instead_of_adding_one() {
    let h = Harness::new();
    h.port().install(
        FakeEventFeed::new()
            .add(1, warning("e", "web-0", "BackOff", 1, 1))
            .add(2, warning("e", "web-0", "BackOff", 4, 2)),
    );
    h.connect().await;
    h.until("watched", |h| h.port().live_feeds() == 1).await;
    h.tick().await;
    h.until("listed", |h| !h.centre.list().is_empty()).await;
    h.tick().await;
    h.until("counted", |h| h.centre.list()[0].count == 4).await;
    assert_eq!(h.centre.list().len(), 1);
}

#[tokio::test]
async fn a_muted_cluster_is_not_watched_and_stays_muted_after_a_restart() {
    let h = Harness::new();
    h.port()
        .install(FakeEventFeed::new().add(1, warning("e", "web-0", "BackOff", 1, 1)));
    h.connect().await;
    h.until("watched", |h| h.centre.is_watching(&cluster()))
        .await;

    assert!(h.centre.mute(&cluster()).await.expect("saved"));
    assert!(
        !h.centre.mute(&cluster()).await.expect("saved"),
        "already muted"
    );
    assert!(h.centre.is_muted(&cluster()));
    assert!(!h.centre.is_watching(&cluster()));
    // A source other than events is dropped too.
    assert!(h.centre.submit(input(&cluster(), "k")).is_none());
    h.tick().await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(h.centre.list().is_empty(), "nothing about a muted cluster");

    // Only the ids are on disk.
    let stored = h
        .state
        .kv_get(&StateKey::new(MUTED_KEY).unwrap())
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(stored, serde_json::json!({ "clusters": [cluster()] }));

    // A restart over the same store: still muted, never watched.
    let restarted = Harness::with_state(h.state.clone());
    restarted.connect().await;
    restarted
        .until("the muted list is read", |r| r.centre.is_muted(&cluster()))
        .await;
    tokio::task::yield_now().await;
    assert!(!restarted.centre.is_watching(&cluster()));

    // Unmuting watches again and is saved.
    assert!(restarted.centre.unmute(&cluster()).await.expect("saved"));
    assert!(restarted.centre.is_watching(&cluster()));
    let stored = restarted
        .state
        .kv_get(&StateKey::new(MUTED_KEY).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored, serde_json::json!({ "clusters": [] }));
}

#[tokio::test]
async fn a_closed_session_is_forgotten() {
    let h = Harness::new();
    h.connect().await;
    h.until("watched", |h| h.centre.is_watching(&cluster()))
        .await;
    h.sessions.close(&cluster());
    h.until("forgotten", |h| !h.centre.is_watching(&cluster()))
        .await;
}

#[tokio::test]
async fn subscribers_get_one_coalesced_item_per_poll() {
    let h = Harness::new();
    let mut sub = h.centre.subscribe();
    next(&mut sub).await;
    for i in 0..100 {
        h.centre.submit(input(&cluster(), &format!("k{i}")));
    }
    let item = next(&mut sub).await;
    assert!(item.changed);
    let rules = h.centre.rules();
    assert_eq!(item.raised.len(), rules.max_raised, "the rate limit holds");
    assert_eq!(item.more_raised, 0);
    assert_eq!(h.centre.unread(), 100);
    // Reading is a change too.
    h.centre.mark_all_read();
    assert!(next(&mut sub).await.changed);
    assert_eq!(h.centre.unread(), 0);
}

#[tokio::test]
async fn raised_notifications_reach_the_os_only_once_opted_in() {
    let h = Harness::new();
    let notifier = Arc::new(FakeNotifierPort::new());
    let os = OsNotificationService::new(notifier.clone());
    tokio::spawn(forward_to_os(h.centre.clone(), os.clone()));
    tokio::task::yield_now().await;

    h.centre.submit(input(&cluster(), "before"));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(notifier.shown().is_empty(), "off: the OS hears nothing");

    os.apply(true).await;
    let mut target = input(&cluster(), "after");
    target.target = Some(warning("e", "web-0", "BackOff", 1, 0).regarding);
    h.centre.submit(target);
    h.until("shown on the desktop", |_| notifier.shown().len() == 1)
        .await;
    let shown = &notifier.shown()[0];
    assert_eq!(shown.title, "title after");
    assert_eq!(shown.tag.as_deref(), Some("after"));
    assert!(shown.action_target.is_some(), "a click opens the object");
}

/// A feed with one Pod warning and one Job warning (`Job` is watched by default; `Service` is not).
fn install_mixed_feed(h: &Harness) {
    h.port().install(
        FakeEventFeed::new()
            .add(1, warning("pod", "web-0", "BackOff", 1, 1))
            .add(2, {
                let mut job = event()
                    .uid("job")
                    .warning()
                    .reason("DeadlineExceeded")
                    .message("too slow")
                    .regarding("Job", Some("default"), "batch", "batch")
                    .build();
                job.last_seen = Some(super::at(2));
                job
            }),
    );
}

#[tokio::test]
async fn new_rules_apply_at_once_and_resubscribe_only_when_the_kinds_change() {
    let h = Harness::new();
    install_mixed_feed(&h);
    h.connect().await;
    h.until("watched", |h| h.port().live_feeds() == 1).await;
    // Every subscription the events service ever handed out takes the next id.
    let opened = |h: &Harness| h.events.inner.state.lock().next_id;
    let before = opened(&h);

    // A window or an ignored reason is no new subscription: the cluster keeps its pump (and so
    // its tracker: nothing already seen is told again).
    let mut rules = h.centre.rules();
    rules.ignored_reasons = vec![Arc::from("Pulled")];
    rules.max_raised = 9;
    h.centre.set_rules(rules.clone());
    assert!(h.centre.is_watching(&cluster()));
    assert_eq!(h.centre.rules().max_raised, 9);
    assert_eq!(opened(&h), before, "no new subscription for a window edit");

    // Narrowing the kinds subscribes again with the new filter; only Jobs notify now.
    rules.watched_kinds = vec![Arc::from("Job")];
    h.centre.set_rules(rules);
    assert_eq!(opened(&h), before + 1, "one new subscription for the kinds");
    h.until("watched again", |h| {
        h.centre.is_watching(&cluster()) && h.port().live_feeds() == 1
    })
    .await;
    h.tick().await;
    h.tick().await;
    h.until("the Job is listed", |h| !h.centre.list().is_empty())
        .await;
    assert_eq!(h.titles(), ["Job default/batch: DeadlineExceeded"]);
}

/// A `Warning` event `uid` about Service `name` in `default` (`Service` is not watched by
/// default), last seen `secs` after `t0`.
fn service_warning(uid: &str, name: &str, secs: i64) -> oxikube_domain::event::Event {
    let mut e = event()
        .uid(uid)
        .warning()
        .reason("SyncLoadBalancerFailed")
        .message("no load balancer")
        .regarding("Service", Some("default"), name, name)
        .build();
    e.last_seen = Some(super::at(secs));
    e
}

#[tokio::test]
async fn widening_the_kinds_does_not_replay_the_newly_watched_kinds_backlog() {
    let h = Harness::new();
    h.port().install(
        FakeEventFeed::new()
            .add(1, warning("pod", "web-0", "BackOff", 1, 1))
            // Not watched yet: it waits in the cluster's buffer, newer than every told event.
            .add(2, service_warning("old-lb", "lb-0", 2))
            // After the kinds change: news.
            .add(4, service_warning("new-lb", "lb-1", 4)),
    );
    h.connect().await;
    h.until("watched", |h| h.port().live_feeds() == 1).await;
    h.tick().await;
    h.until("the Pod is listed", |h| !h.centre.list().is_empty())
        .await;
    h.tick().await;
    h.until("the Service warning is buffered", |h| {
        h.events
            .query(&cluster(), &crate::events::EventFilter::warnings())
            .len()
            == 2
    })
    .await;

    let mut rules = h.centre.rules();
    rules.watched_kinds.push(Arc::from("Service"));
    h.centre.set_rules(rules);
    h.until("watched again", |h| {
        h.centre.is_watching(&cluster()) && h.port().live_feeds() == 1
    })
    .await;
    for _ in 0..2 {
        h.tick().await;
    }
    h.until("the new Service warning is listed", |h| {
        h.centre.list().len() > 1
    })
    .await;
    assert_eq!(
        h.titles(),
        [
            "Service default/lb-1: SyncLoadBalancerFailed",
            "Pod default/web-0: BackOff"
        ],
        "the Service warning held before the change is backlog, not news"
    );
}

#[tokio::test]
async fn what_a_replaced_pump_still_hands_over_is_dropped() {
    let h = Harness::new();
    h.connect().await;
    h.until("watched", |h| {
        h.centre.is_watching(&cluster()) && h.port().live_feeds() == 1
    })
    .await;
    let pump_id = |h: &Harness| h.centre.inner.state.lock().watches[&cluster()].pump_id;
    let old = pump_id(&h);
    let mut rules = h.centre.rules();
    rules.watched_kinds.push(Arc::from("Service"));
    h.centre.set_rules(rules);
    h.until("watched again", |h| h.centre.is_watching(&cluster()))
        .await;
    assert_ne!(pump_id(&h), old, "a new pump");

    // The old pump was between its subscription and the lock when it was replaced.
    let late = crate::events::EventsChanged {
        added: vec![Arc::new(warning("late", "web-0", "BackOff", 1, 1))],
        ..Default::default()
    };
    crate::notifications::driver::on_events(&h.centre.inner, &cluster(), old, late, &[]);
    assert!(
        h.centre.list().is_empty(),
        "the old pump's change is dropped"
    );
}

#[tokio::test]
async fn a_cluster_override_of_the_kinds_applies_to_that_cluster_alone() {
    let h = Harness::new();
    install_mixed_feed(&h);
    h.centre.set_rules(NotificationRules {
        clusters: [(
            cluster(),
            ClusterRules {
                watched_kinds: Some(vec![Arc::from("Pod")]),
                ..ClusterRules::default()
            },
        )]
        .into(),
        ..NotificationRules::default()
    });
    h.connect().await;
    h.until("watched", |h| h.port().live_feeds() == 1).await;
    h.tick().await;
    h.tick().await;
    h.until("the Pod is listed", |h| !h.centre.list().is_empty())
        .await;
    assert_eq!(
        h.titles(),
        ["Pod default/web-0: BackOff"],
        "the Job is not watched here"
    );
}

#[tokio::test]
async fn a_cluster_switched_off_the_desktop_still_notifies_in_the_app() {
    let h = Harness::new();
    let notifier = Arc::new(FakeNotifierPort::new());
    let os = OsNotificationService::new(notifier.clone());
    os.apply(true).await;
    tokio::spawn(forward_to_os(h.centre.clone(), os.clone()));
    tokio::task::yield_now().await;
    h.centre.set_rules(NotificationRules {
        clusters: [(
            cluster(),
            ClusterRules {
                os_notifications: Some(false),
                ..ClusterRules::default()
            },
        )]
        .into(),
        ..NotificationRules::default()
    });
    assert!(!h.centre.os_allowed(&cluster()));

    let quiet = h.centre.submit(input(&cluster(), "quiet")).expect("listed");
    assert!(!quiet.read);
    let loud = h.centre.submit(input(&super::other(), "loud"));
    assert!(loud.is_some());
    h.until("the other cluster reaches the desktop", |_| {
        notifier.shown().len() == 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        notifier.shown().len(),
        1,
        "the switched-off cluster did not"
    );
    assert_eq!(notifier.shown()[0].title, "title loud");
    assert_eq!(h.centre.list().len(), 2, "both are listed in the app");
}

#[tokio::test]
async fn cluster_events_stop_with_their_kind_switched_off() {
    let h = Harness::new();
    h.port().install(
        FakeEventFeed::new()
            .add(1, warning("e1", "web-0", "BackOff", 1, 1))
            .add(2, warning("e2", "web-1", "Failed", 1, 2)),
    );
    h.centre.set_kind_enabled(
        oxikube_domain::notification::NotificationKind::ClusterEvent,
        false,
    );
    h.connect().await;
    h.until("watched", |h| h.port().live_feeds() == 1).await;
    h.tick().await;
    h.tick().await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(h.centre.list().is_empty(), "the kind is off");
}
