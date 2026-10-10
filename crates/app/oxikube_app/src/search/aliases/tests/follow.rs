//! The registry and its follower: one table per cluster, filled from the session's discovery on
//! connect, updated by CRD changes, cleared on disconnect.

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::AliasTarget;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_ports::{ClusterContext, DiscoveryEvent, KindsChange, SourceId};
use oxikube_testkit::kinds::{cert_manager_kinds, core_kinds, kind};
use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort};

use super::gvr;
use crate::search::aliases::{AliasRegistry, AliasTable, Resolution};
use crate::session::ClusterSessionManager;

/// A word only discovery of the stock cluster knows (the built-ins do not).
const STOCK_ONLY: &str = "events.events.k8s.io";

fn ctx(name: &str) -> ClusterContext {
    let context = ContextName::new(name);
    ClusterContext::new(
        ClusterId::new("/home/me/.kube/config", &context),
        context,
        SourceId("kubeconfig".into()),
    )
}

fn id(name: &str) -> ClusterId {
    ctx(name).cluster
}

struct Harness {
    manager: ClusterSessionManager,
    connector: Arc<FakeClusterConnectorPort>,
    registry: AliasRegistry,
    _follow: crate::search::aliases::AliasFollow,
}

impl Harness {
    fn new() -> Self {
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([ctx("a"), ctx("b")]));
        let manager = ClusterSessionManager::new(
            connector.clone(),
            source,
            Arc::new(FakeClockPort::default()),
        );
        let registry = AliasRegistry::new();
        let follow = registry.follow(&manager, &tokio::runtime::Handle::current());
        Self {
            manager,
            connector,
            registry,
            _follow: follow,
        }
    }

    async fn connect(&self, name: &str) {
        self.manager.connect(&id(name)).await.expect("connect");
    }

    /// Waits until `check` holds for `name`'s table; the workers run on other tasks.
    async fn until(&self, name: &str, check: impl Fn(&AliasTable) -> bool) {
        let table = self.registry.table(&id(name));
        for _ in 0..200 {
            if check(&table) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the table of {name} never reached the expected state");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connecting_fills_the_table_from_discovery_and_disconnecting_clears_it() {
    let h = Harness::new();
    h.connector
        .ports_for(&id("a"))
        .discovery
        .set_kinds(cert_manager_kinds());

    assert!(
        !h.registry
            .table(&id("a"))
            .resolve("certificates")
            .is_known()
    );
    h.connect("a").await;
    h.until("a", |t| t.resolve("certificates").is_known()).await;
    assert!(h.registry.table(&id("a")).resolve("po").is_known());

    h.manager.disconnect(&id("a")).expect("disconnect");
    h.until("a", |t| !t.resolve("certificates").is_known())
        .await;
    assert!(
        h.registry.table(&id("a")).resolve("po").is_known(),
        "built-ins stay"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clusters_do_not_share_discovered_aliases() {
    let h = Harness::new();
    h.connector
        .ports_for(&id("a"))
        .discovery
        .set_kinds(cert_manager_kinds());
    let mut b_kinds = core_kinds();
    b_kinds.push(kind("b.io", "v1", "Gizmo", "gizmos").build());
    h.connector.ports_for(&id("b")).discovery.set_kinds(b_kinds);
    h.connect("a").await;
    h.connect("b").await;
    h.until("a", |t| t.resolve("certificates").is_known()).await;
    h.until("b", |t| t.resolve("gizmos").is_known()).await;
    assert!(
        !h.registry
            .table(&id("b"))
            .resolve("certificates")
            .is_known()
    );
    assert!(!h.registry.table(&id("a")).resolve("gizmos").is_known());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_crd_change_updates_only_what_it_names() {
    let h = Harness::new();
    let discovery = h.connector.ports_for(&id("a")).discovery;
    discovery.set_kinds(core_kinds());
    h.connect("a").await;
    h.until("a", |t| t.resolve(STOCK_ONLY).is_known()).await;

    // A CRD appears: the adapter would have re-run discovery; the follower resolves the named
    // kind and does not list everything again.
    let widget = kind("example.io", "v1", "Widget", "widgets")
        .short("wg")
        .build();
    let mut kinds = core_kinds();
    kinds.push(widget.clone());
    discovery.set_kinds(kinds);
    let lists_before = discovery
        .recorded_calls()
        .iter()
        .filter(|c| matches!(c, oxikube_testkit::DiscoveryCall::Discover))
        .count();
    discovery.emit(DiscoveryEvent::KindsChanged(KindsChange {
        added: vec![widget.gvk.clone()],
        ..KindsChange::default()
    }));
    h.until("a", |t| t.resolve("wg").is_known()).await;
    let lists_after = discovery
        .recorded_calls()
        .iter()
        .filter(|c| matches!(c, oxikube_testkit::DiscoveryCall::Discover))
        .count();
    assert_eq!(
        lists_after, lists_before,
        "no full listing for an added kind"
    );

    // And it goes again.
    discovery.set_kinds(core_kinds());
    discovery.emit(DiscoveryEvent::KindsChanged(KindsChange {
        removed: vec![Gvk::new("example.io", "v1", "Widget")],
        ..KindsChange::default()
    }));
    h.until("a", |t| !t.resolve("wg").is_known()).await;
    assert!(h.registry.table(&id("a")).resolve(STOCK_ONLY).is_known());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_without_details_lists_everything_again() {
    let h = Harness::new();
    let discovery = h.connector.ports_for(&id("a")).discovery;
    discovery.set_kinds(core_kinds());
    h.connect("a").await;
    h.until("a", |t| t.resolve(STOCK_ONLY).is_known()).await;

    discovery.set_kinds(cert_manager_kinds());
    discovery.emit(DiscoveryEvent::KindsChanged(KindsChange::default()));
    h.until("a", |t| t.resolve("certificates").is_known()).await;
}

#[test]
fn user_aliases_reach_existing_and_future_tables() {
    let registry = AliasRegistry::new();
    let a = registry.table(&id("a"));
    let target = gvr("", "v1", "pods");
    let shadowing = registry.set_user_aliases(vec![
        ("mine".to_owned(), target.clone()),
        ("po".to_owned(), gvr("", "v1", "nodes")),
    ]);
    assert_eq!(
        shadowing.iter().map(|c| &*c.name).collect::<Vec<_>>(),
        ["po"],
        "the user alias that hides a built-in one is reported"
    );

    assert_eq!(a.resolve("mine").target(), Some(&target));
    let b = registry.table(&id("b"));
    assert_eq!(
        b.resolve("mine").target(),
        Some(&target),
        "a table created later"
    );

    registry.set_user_aliases(Vec::<(String, AliasTarget)>::new());
    assert!(matches!(a.resolve("mine"), Resolution::Unknown { .. }));
    assert!(matches!(
        registry.table(&id("b")).resolve("mine"),
        Resolution::Unknown { .. }
    ));
}

#[test]
fn a_table_is_made_once_per_cluster_and_forgotten_on_request() {
    let registry = AliasRegistry::new();
    let first = registry.table(&id("a"));
    first.set_user_aliases([("x".to_owned(), gvr("", "v1", "pods"))]);
    assert!(
        registry.table(&id("a")).resolve("x").is_known(),
        "the same table"
    );
    registry.forget(&id("a"));
    assert!(
        !registry.table(&id("a")).resolve("x").is_known(),
        "a fresh one"
    );
}
