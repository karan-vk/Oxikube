//! The registry and its follower: one table per cluster, filled from the session's discovery on
//! connect, updated by CRD changes, cleared on disconnect.

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::AliasTarget;
use oxikube_domain::OxiError;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_ports::{ClusterContext, DiscoveryEvent, KindsChange, SourceId};
use oxikube_testkit::kinds::{cert_manager_kinds, core_kinds, kind};
use oxikube_testkit::{
    DiscoveryCall, FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort,
    FakeDiscoveryPort,
};

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

fn count_calls(discovery: &FakeDiscoveryPort, wanted: impl Fn(&DiscoveryCall) -> bool) -> usize {
    discovery
        .recorded_calls()
        .iter()
        .filter(|c| wanted(c))
        .count()
}

/// Waits until the follower has asked about `gvk`. A cluster's worker runs its jobs in order, so
/// every job queued before the one that asks has finished by then. Panics when it never asks, so
/// a test cannot pass by running its assertions before the change under test was applied.
async fn wait_for_resolve(discovery: &FakeDiscoveryPort, gvk: &Gvk) {
    for _ in 0..400 {
        if count_calls(
            discovery,
            |c| matches!(c, DiscoveryCall::Resolve(g) if g == gvk),
        ) > 0
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the follower never resolved {gvk}");
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

/// A CRD served at `v1` (preferred) and `v1beta1`; discovery reports a removal per version.
fn two_version_widget() -> (Vec<oxikube_domain::kinds::ResourceKind>, Gvk) {
    let mut kinds = core_kinds();
    kinds.push(
        kind("example.io", "v1", "Widget", "widgets")
            .short("wg")
            .build(),
    );
    kinds.push(
        kind("example.io", "v1beta1", "Widget", "widgets")
            .short("wg")
            .not_preferred()
            .build(),
    );
    (kinds, Gvk::new("example.io", "v1beta1", "Widget"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_a_non_preferred_version_keeps_the_type() {
    let h = Harness::new();
    let discovery = h.connector.ports_for(&id("a")).discovery;
    let (kinds, beta) = two_version_widget();
    discovery.set_kinds(kinds);
    h.connect("a").await;
    h.until("a", |t| t.resolve("wg").is_known()).await;

    // v1beta1 stops being served; v1 stays. The change names only the removed version.
    let mut kinds = core_kinds();
    kinds.push(
        kind("example.io", "v1", "Widget", "widgets")
            .short("wg")
            .build(),
    );
    discovery.set_kinds(kinds);
    discovery.emit(DiscoveryEvent::KindsChanged(KindsChange {
        removed: vec![beta],
        ..KindsChange::default()
    }));
    // A later, unrelated removal proves the first change has been applied (jobs run in order).
    let gizmo = kind("b.io", "v1", "Gizmo", "gizmos").build();
    discovery.emit(DiscoveryEvent::KindsChanged(KindsChange {
        removed: vec![gizmo.gvk.clone()],
        ..KindsChange::default()
    }));
    wait_for_resolve(&discovery, &gizmo.gvk).await;
    let table = h.registry.table(&id("a"));
    for word in ["wg", "widgets", "widgets.example.io"] {
        let Resolution::Exact(entry) = table.resolve(word) else {
            panic!("`{word}` should still be exact");
        };
        assert_eq!(entry.target, gvr("example.io", "v1", "widgets"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_the_preferred_version_moves_the_type_to_the_one_left() {
    let h = Harness::new();
    let discovery = h.connector.ports_for(&id("a")).discovery;
    let (kinds, _) = two_version_widget();
    discovery.set_kinds(kinds);
    h.connect("a").await;
    h.until("a", |t| t.resolve("wg").is_known()).await;

    // v1 goes; v1beta1 becomes the version the server prefers.
    let mut kinds = core_kinds();
    kinds.push(
        kind("example.io", "v1beta1", "Widget", "widgets")
            .short("wg")
            .build(),
    );
    discovery.set_kinds(kinds);
    discovery.emit(DiscoveryEvent::KindsChanged(KindsChange {
        removed: vec![Gvk::new("example.io", "v1", "Widget")],
        ..KindsChange::default()
    }));
    h.until("a", |t| {
        matches!(t.resolve("wg"), Resolution::Exact(e)
            if matches!(&e.target, AliasTarget::Gvr(g) if &*g.version == "v1beta1"))
    })
    .await;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_listing_keeps_the_tables_as_they_were() {
    let h = Harness::new();
    let discovery = h.connector.ports_for(&id("a")).discovery;
    discovery.set_kinds(core_kinds());
    h.connect("a").await;
    h.until("a", |t| t.resolve(STOCK_ONLY).is_known()).await;

    // The server would now answer differently, but the listing fails: nothing may change.
    discovery.set_kinds(cert_manager_kinds());
    discovery
        .script()
        .discover
        .push_err(OxiError::network("discovery is down"));
    discovery.emit(DiscoveryEvent::KindsChanged(KindsChange::default()));
    // A later job proves the failed listing has been handled (jobs run in order).
    let gizmo = Gvk::new("b.io", "v1", "Gizmo");
    discovery.emit(DiscoveryEvent::KindsChanged(KindsChange {
        removed: vec![gizmo.clone()],
        ..KindsChange::default()
    }));
    wait_for_resolve(&discovery, &gizmo).await;

    let table = h.registry.table(&id("a"));
    assert!(
        table.resolve(STOCK_ONLY).is_known(),
        "a failed listing does not wipe the discovery layer"
    );
    assert!(table.resolve("po").is_known(), "built-ins stay");
    assert!(
        !table.resolve("certificates").is_known(),
        "and the new answer was not applied"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_resolve_lists_everything_again_instead_of_guessing() {
    let h = Harness::new();
    let discovery = h.connector.ports_for(&id("a")).discovery;
    discovery.set_kinds(core_kinds());
    h.connect("a").await;
    h.until("a", |t| t.resolve(STOCK_ONLY).is_known()).await;

    let widget = kind("example.io", "v1", "Widget", "widgets")
        .short("wg")
        .build();
    let mut kinds = core_kinds();
    kinds.push(widget.clone());
    discovery.set_kinds(kinds);
    discovery
        .script()
        .resolve
        .push_err(OxiError::network("discovery is down"));
    let lists_before = count_calls(&discovery, |c| matches!(c, DiscoveryCall::Discover));
    discovery.emit(DiscoveryEvent::KindsChanged(KindsChange {
        added: vec![widget.gvk.clone()],
        ..KindsChange::default()
    }));
    // The added kind is not dropped as "gone": the full listing brings it in.
    h.until("a", |t| t.resolve("wg").is_known()).await;
    assert!(h.registry.table(&id("a")).resolve(STOCK_ONLY).is_known());
    assert_eq!(
        count_calls(&discovery, |c| matches!(c, DiscoveryCall::Discover)),
        lists_before + 1,
        "one full listing replaced the failed resolve"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_resolve_does_not_drop_a_kind_that_is_still_served() {
    let h = Harness::new();
    let discovery = h.connector.ports_for(&id("a"));
    let discovery = discovery.discovery;
    let (kinds, beta) = two_version_widget();
    discovery.set_kinds(kinds);
    h.connect("a").await;
    h.until("a", |t| t.resolve("wg").is_known()).await;

    // The change names a removed version, the resolve fails, and the type is still served.
    discovery
        .script()
        .resolve
        .push_err(OxiError::network("discovery is down"));
    discovery.emit(DiscoveryEvent::KindsChanged(KindsChange {
        removed: vec![beta],
        ..KindsChange::default()
    }));
    let gizmo = Gvk::new("b.io", "v1", "Gizmo");
    discovery.emit(DiscoveryEvent::KindsChanged(KindsChange {
        removed: vec![gizmo.clone()],
        ..KindsChange::default()
    }));
    wait_for_resolve(&discovery, &gizmo).await;
    let table = h.registry.table(&id("a"));
    for word in ["wg", "widgets", "widgets.example.io"] {
        assert!(table.resolve(word).is_known(), "`{word}` must survive");
    }
}
