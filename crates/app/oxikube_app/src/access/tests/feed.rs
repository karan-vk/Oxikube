//! The index kept from a store's four RBAC feeds: the list, the status while it warms or a kind is
//! refused, and the hub that shares one feed and releases the subscriptions with its last user.

use std::sync::Arc;

use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::task::LocalSpawnExt as _;
use oxikube_domain::error::OxiError;
use oxikube_domain::rbac::{RbacKind, Subject};
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort};
use parking_lot::Mutex;
use serde_json::json;

use super::{binding, role};
use crate::access::{PolicyFeed, PolicyHub, PolicyStatus};
use crate::store::{ResourceStore, Spawner, StoreOptions, StorePorts, StoreRuntime};
use crate::testing::id;

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
    port: Arc<FakeResourcePort>,
    store: ResourceStore,
    exec: Executor,
    pool: LocalPool,
}

impl Rig {
    fn new() -> Self {
        let clock = Arc::new(FakeClockPort::default());
        let port = Arc::new(FakeResourcePort::with_clock(clock.clone()));
        let exec = Executor::default();
        let store = ResourceStore::new(
            id("a"),
            StorePorts {
                resources: port.clone(),
                tables: Arc::new(FakeTableFeedPort::with_clock(clock.clone())),
            },
            StoreRuntime {
                spawner: exec.spawner(),
                clock,
                probe: None,
            },
            StoreOptions::default(),
        );
        Self {
            port,
            store,
            exec,
            pool: LocalPool::new(),
        }
    }

    fn settle(&mut self) {
        self.exec.run(&mut self.pool);
    }
}

fn seed(rig: &Rig) {
    rig.port.insert(role(
        None,
        "viewer",
        json!([{"apiGroups": [""], "resources": ["pods"], "verbs": ["get"]}]),
        json!({}),
    ));
    rig.port.insert(binding(
        Some("shop"),
        "ci-views",
        ("ClusterRole", "viewer"),
        json!([{"kind": "ServiceAccount", "name": "ci", "namespace": "shop"}]),
    ));
    rig.port.insert(binding(
        None,
        "ci-admin",
        ("ClusterRole", "viewer"),
        json!([{"kind": "ServiceAccount", "name": "ci", "namespace": "shop"}]),
    ));
}

fn ci() -> Subject {
    Subject::service_account("shop", "ci")
}

#[test]
fn the_index_follows_the_four_rbac_feeds() {
    let mut rig = Rig::new();
    seed(&rig);
    let feed = PolicyFeed::start(&rig.store);
    assert_eq!(feed.status(), PolicyStatus::Warming, "nothing listed yet");
    assert_eq!(feed.version(), 0);

    rig.settle();
    assert_eq!(feed.status(), PolicyStatus::Ready);
    assert!(feed.version() > 0);
    let bindings = feed.read(|index| index.bindings_for(&ci()));
    assert_eq!(bindings.len(), 2, "{bindings:?}");
    assert_eq!(feed.read(|index| index.count(RbacKind::ClusterRole)), 1);
    assert!(feed.reads(&rig.store));
}

#[test]
fn a_kind_the_user_may_not_list_leaves_the_rest_and_says_so() {
    let mut rig = Rig::new();
    seed(&rig);
    // Roles are listed first, ClusterRoles second: the second watch is refused.
    rig.port
        .script()
        .watch
        .push_err(OxiError::forbidden("roles is forbidden"));
    let feed = PolicyFeed::start(&rig.store);
    rig.settle();
    match feed.status() {
        PolicyStatus::Partial(missing) => {
            assert_eq!(missing.len(), 1, "{missing:?}");
            assert!(missing[0].1.contains("forbidden"), "{missing:?}");
        }
        other => panic!("expected a partial answer, got {other:?}"),
    }
}

#[test]
fn the_hub_shares_one_feed_and_releases_it_with_the_last_user() {
    let mut rig = Rig::new();
    seed(&rig);
    let hub = PolicyHub::default();
    let cluster = id("a");
    assert_eq!(hub.live(&cluster), 0, "nothing runs until a view asks");

    let first = hub.acquire(&rig.store);
    let second = hub.acquire(&rig.store);
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(hub.live(&cluster), 1);
    rig.settle();
    assert_eq!(rig.port.live_watches(), 4, "one watch per RBAC kind");

    drop(first);
    assert_eq!(hub.live(&cluster), 1, "still held by the second user");
    drop(second);
    assert_eq!(hub.live(&cluster), 0);
}
