//! The index fed by a real `ResourceStore` over `FakeResourcePort` scripts: snapshots, then a
//! binding deleted by a watch delta.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::task::LocalSpawnExt as _;
use oxikube_domain::Resource;
use oxikube_domain::access::Access;
use oxikube_domain::ids::{ClusterId, ContextName, Gvr};
use oxikube_domain::rbac::{RbacKind, Subject};
use oxikube_domain::session::WatchScope;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort, Timeline};
use parking_lot::Mutex;
use serde_json::json;

use super::{binding, role};
use crate::access::SubjectIndex;
use crate::store::{
    ResourceStore, RowChange, RowOp, Spawner, StoreObject, StoreOptions, StorePorts, StoreQuery,
    StoreRuntime, Subscription,
};

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
    clock: Arc<FakeClockPort>,
    resources: Arc<FakeResourcePort>,
    store: ResourceStore,
    exec: Executor,
    pool: LocalPool,
    subscriptions: Vec<(RbacKind, Subscription)>,
    index: SubjectIndex,
}

impl Rig {
    fn new() -> Self {
        let clock = Arc::new(FakeClockPort::default());
        let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
        let exec = Executor::default();
        let store = ResourceStore::new(
            ClusterId::new("test", &ContextName::from("kind")),
            StorePorts {
                resources: resources.clone(),
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
            clock,
            resources,
            store,
            exec,
            pool: LocalPool::new(),
            subscriptions: Vec::new(),
            index: SubjectIndex::new(),
        }
    }

    /// Subscribes to `kind` cluster-wide (the next scripted watch, else the seeded objects).
    fn follow(&mut self, kind: RbacKind) {
        let subscription = self
            .store
            .subscribe(StoreQuery::new(kind.gvk(), WatchScope::Cluster));
        self.subscriptions.push((kind, subscription));
        self.settle();
    }

    /// Runs the store and feeds every pending subscription item to the index.
    fn settle(&mut self) {
        self.exec.run(&mut self.pool);
        for (kind, subscription) in &mut self.subscriptions {
            while let Some(Some(delta)) = subscription.next().now_or_never() {
                self.index.apply(*kind, &delta);
            }
        }
    }

    fn advance(&mut self, secs: u64) {
        self.clock.advance(Duration::from_secs(secs));
        self.settle();
    }
}

fn reader() -> Resource {
    role(
        None,
        "reader",
        json!([{"apiGroups": [""], "resources": ["pods"], "verbs": ["get", "list"]}]),
        json!({}),
    )
}

fn grant(namespace: &str, user: &str) -> Resource {
    binding(
        Some(namespace),
        &format!("{user}-reads"),
        ("ClusterRole", "reader"),
        json!([{"kind": "User", "name": user}]),
    )
}

#[test]
fn deleting_a_binding_shrinks_only_its_subjects_rules() {
    let mut rig = Rig::new();
    let alice_shop = grant("shop", "alice");
    let alice_web = grant("web", "alice");
    let bob = grant("shop", "bob");
    // The RoleBindings feed: the list, then alice's `shop` binding deleted a second later.
    rig.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::ZERO,
                DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
                    alice_shop.clone(),
                    alice_web,
                    bob,
                ])]),
            )
            .ok_at(
                Duration::from_secs(1),
                DeltaBatch::from_deltas(vec![Delta::Deleted(alice_shop)]),
            )
            .keep_open(),
    );
    rig.follow(RbacKind::RoleBinding);
    // The other kinds list what the fake holds.
    rig.resources.insert(reader());
    for kind in [
        RbacKind::Role,
        RbacKind::ClusterRole,
        RbacKind::ClusterRoleBinding,
    ] {
        rig.follow(kind);
    }

    let alice = Subject::User("alice".into());
    let bob = Subject::User("bob".into());
    let pods = Gvr::new("", "v1", "pods");
    let before_alice = rig.index.rules_for(&alice);
    let before_bob = rig.index.rules_for(&bob);
    let subjects = rig.index.subjects();
    assert_eq!(before_alice.len(), 2, "shop and web");
    assert_eq!(
        rig.index.can(&alice, "list", &pods, Some("shop"), None),
        Access::Granted
    );
    let version = rig.index.version();

    rig.advance(1);
    let after_alice = rig.index.rules_for(&alice);
    assert_eq!(after_alice.len(), 1, "rules_for shrank");
    assert_eq!(
        after_alice[0], before_alice[1],
        "the web grant is untouched"
    );
    assert_eq!(
        rig.index.can(&alice, "list", &pods, Some("shop"), None),
        Access::Denied
    );
    assert_eq!(
        rig.index.rules_for(&bob),
        before_bob,
        "nothing else changed"
    );
    assert_eq!(rig.index.subjects(), subjects);
    assert_eq!(rig.index.count(RbacKind::RoleBinding), 2);
    assert_eq!(rig.index.count(RbacKind::ClusterRole), 1);
    assert_eq!(rig.index.version(), version + 1, "one change, one version");
}

#[test]
fn ops_edit_one_object_each_and_unchanged_items_change_nothing() {
    let mut index = SubjectIndex::new();
    let object = |resource: Resource| Arc::new(StoreObject::Resource(resource));
    index.apply_rows(
        RbacKind::ClusterRole,
        &RowChange::Snapshot(vec![object(reader())]),
    );
    let alice = Subject::User("alice".into());
    let insert = |at: usize, resource: Resource| RowOp::Insert {
        index: at,
        object: object(resource),
    };
    assert!(index.apply_rows(
        RbacKind::RoleBinding,
        &RowChange::Ops(vec![
            insert(0, grant("shop", "alice")),
            insert(1, grant("web", "alice"))
        ]),
    ));
    assert_eq!(index.bindings_for(&alice).len(), 2);
    let version = index.version();
    assert!(!index.apply_rows(RbacKind::RoleBinding, &RowChange::Unchanged));
    let same = RowOp::Update {
        index: 0,
        object: object(grant("shop", "alice")),
    };
    assert!(!index.apply_rows(RbacKind::RoleBinding, &RowChange::Ops(vec![same])));
    assert_eq!(
        index.version(),
        version,
        "an identical update is not a change"
    );
    let out_of_step = RowOp::Remove { index: 9 };
    assert!(!index.apply_rows(RbacKind::RoleBinding, &RowChange::Ops(vec![out_of_step])));
    assert!(index.apply_rows(
        RbacKind::RoleBinding,
        &RowChange::Ops(vec![RowOp::Remove { index: 0 }]),
    ));
    assert_eq!(index.bindings_for(&alice).len(), 1);
    // A relist of the kind replaces it whole.
    assert!(index.apply_rows(RbacKind::RoleBinding, &RowChange::Snapshot(Vec::new())));
    assert!(index.subjects().is_empty());
    assert_eq!(
        index.count(RbacKind::ClusterRole),
        1,
        "other kinds keep their objects"
    );
}
