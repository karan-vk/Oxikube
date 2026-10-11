//! The index kept from a store's Pods feed: the list, live updates, the namespace selection and
//! teardown.

use std::sync::Arc;
use std::time::Duration;

use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::task::LocalSpawnExt as _;
use oxikube_domain::Resource;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::storage::pod_mounting;
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort, Timeline};
use parking_lot::Mutex;

use super::CLUSTER;
use crate::storage::{PvcUsage, PvcUsageFeed};
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

#[test]
fn the_index_follows_the_pods_feed() {
    let mut rig = Rig::new();
    rig.pods.script().watch.push_ok(timeline(vec![
        vec![Delta::Restarted(vec![
            pod_mounting("web-0", "shop", &["data"]),
            pod_mounting("web-1", "shop", &["data"]),
        ])],
        vec![
            Delta::Applied(pod_mounting("web-2", "shop", &["data", "logs"])),
            Delta::Deleted(pod_mounting("web-0", "shop", &["data"])),
        ],
    ]));
    let feed = PvcUsageFeed::start(&rig.store, &rig.sessions.manager);
    let index = feed.index().clone();
    assert_eq!(
        index.count("shop", "data"),
        None,
        "nothing is known before the list"
    );

    rig.settle();
    assert_eq!(index.count("shop", "data"), Some(2));
    assert!(feed.reads(&rig.store));

    rig.advance(1);
    assert_eq!(index.count("shop", "data"), Some(2));
    assert_eq!(index.count("shop", "logs"), Some(1));
    let users: Vec<String> = index
        .users("shop", "data")
        .iter()
        .map(|p| p.to_string())
        .collect();
    assert_eq!(users, ["web-1", "web-2"]);
}

#[test]
fn dropping_the_feed_stops_following() {
    let mut rig = Rig::new();
    rig.pods.script().watch.push_ok(timeline(vec![
        vec![Delta::Restarted(vec![pod_mounting(
            "web-0",
            "shop",
            &["data"],
        )])],
        vec![Delta::Applied(pod_mounting("web-1", "shop", &["data"]))],
    ]));
    let feed = PvcUsageFeed::start(&rig.store, &rig.sessions.manager);
    let index = feed.index().clone();
    rig.settle();
    assert_eq!(index.count("shop", "data"), Some(1));
    drop(feed);
    rig.advance(1);
    assert_eq!(
        index.count("shop", "data"),
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
    let feed = PvcUsageFeed::start(&rig.store, &rig.sessions.manager);
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
