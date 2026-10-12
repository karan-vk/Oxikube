//! The index kept from a store's Pods feed: the list, live updates, the namespace selection,
//! teardown, and the hub that shares one feed and releases the Pods subscription with its last
//! user.

use std::sync::Arc;
use std::time::Duration;

use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::task::LocalSpawnExt as _;
use oxikube_domain::Resource;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::view::RefKind;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort, Timeline};
use parking_lot::Mutex;

use super::{CLUSTER, pod_cm};
use crate::references::{ReferenceFeed, ReferenceHub};
use crate::store::{ResourceStore, Spawner, StoreOptions, StorePorts, StoreRuntime};
use crate::testing::{Harness, id};

/// Runs the store's tasks on a `LocalPool`, only when the test says so.
#[derive(Clone, Default)]
struct Executor {
    queue: Arc<Mutex<Vec<BoxFuture<'static, ()>>>>,
}

impl Executor {
    fn spawner(&self) -> Arc<dyn Spawner> {
        let queue = self.queue.clone();
        Arc::new(move |task: BoxFuture<'static, ()>| queue.lock().push(task))
    }

    fn run(&self, pool: &mut LocalPool) {
        loop {
            let tasks = std::mem::take(&mut *self.queue.lock());
            for task in tasks {
                pool.spawner().spawn_local(task).expect("spawn");
            }
            pool.run_until_stalled();
            if self.queue.lock().is_empty() {
                return;
            }
        }
    }
}

struct Rig {
    sessions: Harness,
    clock: Arc<FakeClockPort>,
    pods: Arc<FakeResourcePort>,
    store: ResourceStore,
    exec: Executor,
    pool: LocalPool,
}

impl Rig {
    fn new() -> Self {
        let sessions = Harness::new();
        sessions.connect(CLUSTER, false);
        let clock = Arc::new(FakeClockPort::default());
        let pods = Arc::new(FakeResourcePort::with_clock(clock.clone()));
        let exec = Executor::default();
        let store = ResourceStore::new(
            id(CLUSTER),
            StorePorts {
                resources: pods.clone(),
                tables: Arc::new(FakeTableFeedPort::with_clock(clock.clone())),
            },
            StoreRuntime {
                spawner: exec.spawner(),
                clock: clock.clone(),
                probe: None,
            },
            StoreOptions::default(),
        );
        Self {
            sessions,
            clock,
            pods,
            store,
            exec,
            pool: LocalPool::new(),
        }
    }

    fn settle(&mut self) {
        self.exec.run(&mut self.pool);
    }

    fn advance(&mut self, secs: u64) {
        self.clock.advance(Duration::from_secs(secs));
        self.settle();
    }
}

fn timeline(items: Vec<Vec<Delta<Resource>>>) -> Timeline<DeltaBatch<Resource>> {
    items
        .into_iter()
        .enumerate()
        .fold(Timeline::new(), |t, (i, deltas)| {
            t.ok_at(
                Duration::from_secs(i as u64),
                DeltaBatch::from_deltas(deltas),
            )
        })
        .keep_open()
}

const SHOP: Option<&str> = Some("shop");

#[test]
fn the_index_follows_the_pods_feed() {
    let mut rig = Rig::new();
    rig.pods.script().watch.push_ok(timeline(vec![
        vec![Delta::Restarted(vec![
            pod_cm("web-0", "shop", "cfg"),
            pod_cm("web-1", "shop", "cfg"),
        ])],
        vec![
            Delta::Applied(pod_cm("web-2", "shop", "cfg")),
            Delta::Deleted(pod_cm("web-0", "shop", "cfg")),
        ],
    ]));
    let feed = ReferenceFeed::start(&rig.store, &rig.sessions.manager);
    let index = feed.index().clone();
    assert_eq!(
        index.count(RefKind::ConfigMap, SHOP, "cfg"),
        None,
        "nothing is known before the list"
    );

    rig.settle();
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), Some(2));
    assert!(feed.reads(&rig.store));

    rig.advance(1);
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), Some(2));
    let users: Vec<String> = match index.usage(RefKind::ConfigMap, SHOP, "cfg") {
        crate::references::Usage::Pods { pods, .. } => {
            pods.iter().map(|p| p.pod.name.to_string()).collect()
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(users, ["web-1", "web-2"]);
}

#[test]
fn dropping_the_feed_stops_following() {
    let mut rig = Rig::new();
    rig.pods.script().watch.push_ok(timeline(vec![
        vec![Delta::Restarted(vec![pod_cm("web-0", "shop", "cfg")])],
        vec![Delta::Applied(pod_cm("web-1", "shop", "cfg"))],
    ]));
    let feed = ReferenceFeed::start(&rig.store, &rig.sessions.manager);
    let index = feed.index().clone();
    rig.settle();
    assert_eq!(index.count(RefKind::ConfigMap, SHOP, "cfg"), Some(1));
    drop(feed);
    rig.advance(1);
    assert_eq!(
        index.count(RefKind::ConfigMap, SHOP, "cfg"),
        Some(1),
        "the aborted task applied nothing more"
    );
}

#[test]
fn the_pods_follow_the_namespace_selection() {
    let mut rig = Rig::new();
    let pods = rig.pods.clone();
    let scopes = move || {
        pods.recorded_calls()
            .iter()
            .filter_map(|call| match call {
                oxikube_testkit::ResourceCall::Watch { namespace, .. } => Some(namespace.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let feed = ReferenceFeed::start(&rig.store, &rig.sessions.manager);
    rig.settle();
    assert_eq!(scopes(), [None], "all namespaces to begin with");

    rig.sessions
        .manager
        .set_namespace_selection(&id(CLUSTER), NamespaceSelection::single("shop"))
        .expect("selects");
    rig.settle();
    assert!(
        scopes().contains(&Some("shop".to_owned())),
        "the Pods moved to the selected namespace: {:?}",
        scopes()
    );
    drop(feed);
}

#[test]
fn the_index_covers_only_the_selected_namespaces() {
    let mut rig = Rig::new();
    let feed = ReferenceFeed::start(&rig.store, &rig.sessions.manager);
    let index = feed.index().clone();
    rig.settle();
    assert!(index.covers("shop"), "all namespaces to begin with");

    rig.sessions
        .manager
        .set_namespace_selection(&id(CLUSTER), NamespaceSelection::single("default"))
        .expect("selects");
    rig.settle();
    assert!(index.covers("default"));
    assert!(!index.covers("shop"));
    assert_eq!(
        index.count(RefKind::ConfigMap, SHOP, "cfg"),
        None,
        "an object outside the selection has unknown, not zero, users"
    );
    drop(feed);
}

#[test]
fn the_hub_shares_one_feed_and_releases_the_pods_with_the_last_user() {
    let mut rig = Rig::new();
    rig.pods
        .script()
        .watch
        .push_ok(timeline(vec![vec![Delta::Restarted(vec![pod_cm(
            "web-0", "shop", "cfg",
        )])]]));
    let hub = ReferenceHub::new();
    let cluster = id(CLUSTER);
    assert_eq!(hub.live(&cluster), 0, "nothing runs until a view asks");

    // Two views (a "Used by" tab and the claims table) share one feed and one subscription.
    let first = hub.acquire(&rig.store, &rig.sessions.manager);
    let second = hub.acquire(&rig.store, &rig.sessions.manager);
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(hub.live(&cluster), 1);
    rig.settle();
    assert_eq!(rig.pods.live_watches(), 1);
    assert_eq!(
        first.index().count(RefKind::ConfigMap, SHOP, "cfg"),
        Some(1)
    );

    // One tab closes: the other still has its answers.
    drop(first);
    rig.advance(120);
    assert_eq!(rig.pods.live_watches(), 1, "still held by the second user");
    assert_eq!(
        second.index().count(RefKind::ConfigMap, SHOP, "cfg"),
        Some(1)
    );

    // The last one closes: the Pods subscription is released and the feed stops after the
    // store's grace period.
    drop(second);
    assert_eq!(hub.live(&cluster), 0);
    rig.settle();
    rig.advance(31);
    assert_eq!(rig.pods.live_watches(), 0, "the Pods feed ref is released");

    // A tab opened later starts it again.
    rig.pods
        .script()
        .watch
        .push_ok(timeline(vec![vec![Delta::Restarted(vec![])]]));
    let again = hub.acquire(&rig.store, &rig.sessions.manager);
    rig.settle();
    assert_eq!(rig.pods.live_watches(), 1);
    assert_eq!(
        again.index().count(RefKind::ConfigMap, SHOP, "cfg"),
        Some(0)
    );
}

#[test]
fn a_feed_over_a_replaced_store_is_not_reused() {
    let rig = Rig::new();
    let other = Rig::new();
    let hub = ReferenceHub::new();
    let a = hub.acquire(&rig.store, &rig.sessions.manager);
    let b = hub.acquire(&other.store, &other.sessions.manager);
    assert!(
        !Arc::ptr_eq(&a, &b),
        "a reconnect's new store gets its own feed"
    );
    assert!(a.reads(&rig.store) && !a.reads(&other.store));
    assert_eq!(hub.live(&id(CLUSTER)), 2);
}
