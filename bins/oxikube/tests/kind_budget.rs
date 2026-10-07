//! The watch budget in the app path, against kind, with two clusters (E04-F543).
//!
//! The app's own wiring, without a window: the kube connector, the session manager over it, the
//! resource stores with the options the main window gives them
//! (`oxikube::kube_ports::WatchBudgets::store_options`) and the per-cluster `watch_budget`
//! settings pushed the way a settings reload pushes them (`WatchBudgets::apply`). Views
//! subscribe to the stores as tables and sidebar counts do; every feed they cause is a feed of
//! the connection's `FeedRegistry`, so its counters show:
//!
//! * limits: a cluster with `max_feeds: 2` refuses a third view's feed with `BudgetExceeded`
//!   while the other cluster, on the defaults, opens all three;
//! * eviction and idle teardown: a closed view's feed stays for the grace period, is closed
//!   first when the budget is full, and is torn down when the grace period ends;
//! * re-scoping: moving a view from one namespace to another moves its feed;
//! * hot reload: a changed `watch_budget` applies to live connections (a tighter limit, and a
//!   full kind opened metadata-only past `metadata_above`).
//!
//! The two clusters are two kubeconfig contexts of the one kind cluster (in memory: nothing is
//! written to disk); sessions, connections and budgets are per context. Every feed is scoped to
//! the test's own `oxi-test-<rand>` namespaces, which hold only what Kubernetes puts in a new
//! namespace (the `kube-root-ca.crt` ConfigMap and the `default` ServiceAccount).
//!
//! `OXIKUBE_TEST_CONTEXT=kind-oxikube cargo test -p oxikube --features integration --test
//! kind_budget`; without the variable the test returns at once.
#![cfg(feature = "integration")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use oxikube::kube_ports::{SystemClock, WatchBudgets};
use oxikube_app::session::{SessionManagerConfig, SessionOptions};
use oxikube_app::store::{FeedKind, FeedState, StoreQuery, StoreRuntime, Subscription};
use oxikube_app::{ClusterSessionManager, ResourceStore, ResourceStores};
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::session::{SessionPhase, WatchScope};
use oxikube_kube::kubeconfig::{Strictness, default_kubeconfig_path, load_local_kubeconfig};
use oxikube_kube::{ConnectorConfig, ContextDefinition, KubeConnector, PoolConfig};
use oxikube_ports::{
    ClockPort, ClusterContext, ClusterPrefs, ClusterPrefsTable, FeedStats, FeedVariant, SourceId,
    WatchBudgetPrefs,
};
use oxikube_testkit::FakeClusterSourcePort;
use oxikube_testkit::integration::{TestNamespace, test_context};

/// Short, so the test sees closed views' feeds torn down.
const GRACE: Duration = Duration::from_secs(3);
/// Far longer than kind needs; a hang fails here.
const DEADLINE: Duration = Duration::from_secs(60);

fn config_maps() -> Gvk {
    Gvk::new("", "v1", "ConfigMap")
}

fn service_accounts() -> Gvk {
    Gvk::new("", "v1", "ServiceAccount")
}

fn roles() -> Gvk {
    Gvk::new("rbac.authorization.k8s.io", "v1", "Role")
}

fn in_ns(gvk: Gvk, ns: &TestNamespace) -> StoreQuery {
    StoreQuery::new(gvk, WatchScope::Namespaces(vec![ns.name().to_owned()]))
}

fn budget(max_feeds: usize, metadata_above: u64) -> ClusterPrefs {
    ClusterPrefs {
        watch_budget: WatchBudgetPrefs {
            max_feeds,
            metadata_above,
            idle_grace: GRACE,
            ..WatchBudgetPrefs::default()
        },
        ..ClusterPrefs::default()
    }
}

/// The kind context alone, plus a second context `second` on the same cluster and user.
async fn two_contexts(context: &ContextName, second: &ContextName) -> kube::config::Kubeconfig {
    let home = std::env::var_os("HOME").map(|h| default_kubeconfig_path(&PathBuf::from(h)));
    let loaded = load_local_kubeconfig(
        std::env::var_os("KUBECONFIG"),
        home,
        Strictness::RequireUsable,
    )
    .await
    .expect("load the local kubeconfig");
    let mut kubeconfig = ContextDefinition::from_kubeconfig(&loaded.merged, context)
        .expect("the kind context")
        .kubeconfig()
        .clone();
    let mut named = kubeconfig.contexts[0].clone();
    named.name = second.to_string();
    kubeconfig.contexts.push(named);
    kubeconfig
}

/// Polls `check` until it holds, failing with `diagnostics` after [`DEADLINE`].
async fn eventually(what: &str, diagnostics: impl Fn() -> String, mut check: impl FnMut() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(
            started.elapsed() < DEADLINE,
            "timed out waiting for {what}\n{}",
            diagnostics()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn stats(budgets: &WatchBudgets, cluster: &ClusterId) -> FeedStats {
    budgets
        .registry(cluster)
        .expect("a live connection")
        .stats()
}

/// The (kind, namespace, variant) of every feed of a cluster.
fn feeds(
    budgets: &WatchBudgets,
    cluster: &ClusterId,
) -> Vec<(String, Option<String>, FeedVariant)> {
    stats(budgets, cluster)
        .per_feed
        .into_iter()
        .map(|f| (f.gvk.kind.to_string(), f.namespace, f.variant))
        .collect()
}

fn report(step: &str, stats: &FeedStats) {
    eprintln!(
        "{step}: {}",
        oxikube::kube_ports::report_line("cluster", stats)
    );
}

fn is_refused(sub: &Subscription) -> bool {
    matches!(
        sub.state(),
        FeedState::Failed {
            kind: ErrorKind::BudgetExceeded,
            ..
        }
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_apps_feeds_are_budgeted_per_cluster() {
    let Some(context) = test_context() else {
        return;
    };
    let ns1 = TestNamespace::create(&context).expect("namespace 1");
    let ns2 = TestNamespace::create(&context).expect("namespace 2");
    let a_context = ContextName::new(context.clone());
    let b_context = ContextName::new(format!("oxi-budget-{}", ns1.name()));
    let kubeconfig = two_contexts(&a_context, &b_context).await;

    // --- the app's wiring --------------------------------------------------------------------
    let kube = KubeConnector::new(
        kubeconfig,
        PoolConfig::default(),
        ConnectorConfig::default(),
    );
    let budgets = WatchBudgets::new(&kube);
    let a = ClusterContext::new(
        ClusterId::new("kind-budget", &a_context),
        a_context.clone(),
        SourceId("kind-budget".into()),
    );
    let b = ClusterContext::new(
        ClusterId::new("kind-budget", &b_context),
        b_context.clone(),
        SourceId("kind-budget".into()),
    );
    let settings = |a_prefs: ClusterPrefs| {
        ClusterPrefsTable::new(budget(64, 25_000)).with_cluster(a.cluster.clone(), a_prefs)
    };
    budgets.apply(settings(budget(2, 25_000)));
    let clock: Arc<dyn ClockPort> = Arc::new(SystemClock::new(tokio::runtime::Handle::current()));
    let manager = ClusterSessionManager::with_config(
        Arc::new(kube.clone()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
        SessionManagerConfig::default(),
    );
    let handle = tokio::runtime::Handle::current();
    let runtime = StoreRuntime {
        spawner: Arc::new(move |task: BoxFuture<'static, ()>| {
            handle.spawn(task);
        }),
        clock,
        probe: None,
    };
    let stores = ResourceStores::with_options(runtime, budgets.store_options());

    for entry in [&a, &b] {
        manager.open(entry, SessionOptions::default());
        let state = manager.connect(&entry.cluster).await.expect("connect");
        assert_eq!(
            state.phase(),
            SessionPhase::Ready,
            "{}: {state:?}",
            entry.context
        );
    }
    let store = |entry: &ClusterContext| -> ResourceStore {
        let session = manager.get(&entry.cluster).expect("a session");
        stores.for_session(&session).expect("a connected store")
    };
    let (store_a, store_b) = (store(&a), store(&b));
    let (a_id, b_id) = (a.cluster.clone(), b.cluster.clone());
    let diag = || {
        format!(
            "a: {:?}\n   {:?}\nb: {:?}\n   {:?}",
            budgets.registry(&a_id).map(|r| r.stats()),
            store_a.feeds(),
            budgets.registry(&b_id).map(|r| r.stats()),
            store_b.feeds(),
        )
    };

    // --- limits: two feeds on a, three on b ----------------------------------------------------
    let a_cm = store_a.subscribe(in_ns(config_maps(), &ns1));
    let a_sa = store_a.subscribe(in_ns(service_accounts(), &ns1));
    eventually("a's two views are ready", diag, || {
        a_cm.state().is_ready() && a_sa.state().is_ready()
    })
    .await;
    let mut a_roles = store_a.subscribe(in_ns(roles(), &ns1));
    eventually("a's third view is refused", diag, || is_refused(&a_roles)).await;
    let refused = stats(&budgets, &a_id);
    report("a, limit 2", &refused);
    assert_eq!((refused.feeds, refused.refused), (2, 1), "{}", diag());

    let mut b_cm = store_b.subscribe(in_ns(config_maps(), &ns1));
    let b_sa = store_b.subscribe(in_ns(service_accounts(), &ns1));
    let b_roles = store_b.subscribe(in_ns(roles(), &ns1));
    eventually("b's three views are ready", diag, || {
        [&b_cm, &b_sa, &b_roles]
            .iter()
            .all(|s| s.state().is_ready())
    })
    .await;
    report("b, defaults", &stats(&budgets, &b_id));
    assert_eq!(stats(&budgets, &b_id).feeds, 3, "b's budget is its own");
    // The namespace's controllers create its ConfigMap and ServiceAccount a moment after it.
    eventually(
        "b holds the namespace's ConfigMap and ServiceAccount",
        diag,
        || stats(&budgets, &b_id).objects == 2,
    )
    .await;

    // --- eviction: a closed view's feed makes room ----------------------------------------------
    drop(a_sa);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(stats(&budgets, &a_id).feeds, 2, "kept for its grace period");
    a_roles.retry();
    eventually("the refused view opens in the room", diag, || {
        a_roles.state().is_ready()
    })
    .await;
    eventually("a holds its limit", diag, || {
        let kinds: Vec<String> = feeds(&budgets, &a_id).into_iter().map(|f| f.0).collect();
        kinds == ["ConfigMap", "Role"]
    })
    .await;

    // --- idle teardown -------------------------------------------------------------------------
    drop(a_cm);
    drop(a_roles);
    let closed = Instant::now();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        stats(&budgets, &a_id).feeds,
        2,
        "still within the grace period"
    );
    eventually("a's idle feeds are torn down", diag, || {
        stats(&budgets, &a_id).feeds == 0
    })
    .await;
    assert!(
        closed.elapsed() >= GRACE,
        "not before the grace period ended"
    );
    let torn = stats(&budgets, &a_id);
    report("a, idle", &torn);
    assert_eq!(torn.started, torn.stopped);

    // --- re-scoping: b's ConfigMap view moves from ns1 to ns2 ------------------------------------
    b_cm.rescope(WatchScope::Namespaces(vec![ns2.name().to_owned()]));
    eventually("b's ConfigMap feed moved to ns2", diag, || {
        let cm: Vec<Option<String>> = feeds(&budgets, &b_id)
            .into_iter()
            .filter(|f| f.0 == "ConfigMap")
            .map(|f| f.1)
            .collect();
        cm == [Some(ns2.name().to_owned())]
    })
    .await;
    eventually("b's rescoped view is ready", diag, || {
        b_cm.state().is_ready()
    })
    .await;
    report("b, rescoped", &stats(&budgets, &b_id));
    assert_eq!(
        stats(&budgets, &b_id).feeds,
        3,
        "re-scoped, not accumulated"
    );

    // --- hot reload: a tighter limit, then metadata-only past the threshold ---------------------
    budgets.apply(settings(budget(1, 25_000)));
    assert_eq!(budgets.registry(&a_id).unwrap().config().max_feeds, 1);
    let one = store_a.subscribe(in_ns(config_maps(), &ns2));
    eventually("a's one view is ready", diag, || one.state().is_ready()).await;
    let two = store_a.subscribe(in_ns(roles(), &ns2));
    eventually("a second view is refused at the new limit", diag, || {
        is_refused(&two)
    })
    .await;
    drop((one, two));

    budgets.apply(settings(budget(4, 0)));
    let degraded = store_a.subscribe(in_ns(service_accounts(), &ns2));
    eventually("the full kind opens metadata-only", diag, || {
        degraded.state().is_ready()
    })
    .await;
    assert_eq!(degraded.feed_kind(), FeedKind::Metadata);
    assert_eq!(stats(&budgets, &a_id).degraded, 1);
    assert!(
        feeds(&budgets, &a_id)
            .iter()
            .any(|f| f.0 == "ServiceAccount" && f.2 == FeedVariant::Metadata),
        "{}",
        diag()
    );
    report("a, metadata above 0", &stats(&budgets, &a_id));

    drop((degraded, b_cm, b_sa, b_roles));
    for entry in [&a, &b] {
        manager.disconnect(&entry.cluster).expect("disconnect");
    }
}
