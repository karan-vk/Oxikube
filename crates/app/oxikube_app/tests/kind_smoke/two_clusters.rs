//! The acceptance scenario: two contexts connect through the real adapter and the app session
//! service, and changing the namespace selection of one switches its feeds and nothing else.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxikube_app::session::namespaces::{NamespaceService, NamespaceSource};
use oxikube_app::session::{
    ClusterSession, ClusterSessionManager, SessionManagerConfig, SessionOptions,
};
use oxikube_domain::Capabilities;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::{NamespaceSelection, SessionPhase};
use oxikube_kube::{BudgetConfig, ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_ports::{ClusterContext, FeedStats};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry, create_pods, read_only_pod_viewer};
use crate::eventually;
use crate::scoped_feeds::ScopedPods;

/// Short enough that the test sees the budget tear a released feed down.
const IDLE_GRACE: Duration = Duration::from_secs(2);
const A_PODS: usize = 3;
const B_PODS: usize = 2;

fn pods_in(namespace: &TestNamespace, prefix: &str, count: usize) -> BTreeSet<String> {
    (0..count)
        .map(|i| format!("{}/{prefix}-{i}", namespace.name()))
        .collect()
}

/// Connects `entry` and prints how long it took to reach `Ready`.
async fn connect(manager: &ClusterSessionManager, entry: &ClusterContext) {
    let started = Instant::now();
    let state = manager.connect(&entry.cluster).await.expect("connect");
    let took = started.elapsed();
    eprintln!(
        "connect `{}`: {:?} in {took:?}",
        entry.context,
        state.phase()
    );
    assert_eq!(
        state.phase(),
        SessionPhase::Ready,
        "{}: {state:?}",
        entry.context
    );
    // The cluster-open budget (docs/PERFORMANCE.md) is 200 ms after connect for a local cluster;
    // CI runners are slower, so this only catches a connect that has gone wrong by an order of magnitude.
    assert!(
        took < Duration::from_secs(5),
        "{}: {took:?} to Ready",
        entry.context
    );
}

/// Session states and counters of both clusters, for a failed wait.
fn diagnostics(manager: &ClusterSessionManager, connector: &KubeConnector) -> String {
    let mut out = String::new();
    for session in manager.sessions() {
        let feeds = connector.feeds(session.id()).map(|r| r.stats());
        out += &format!(
            "session `{}`: {:?}, selection {:?}, capabilities {:?}\n  feeds: {:?}\n",
            session.context(),
            session.state(),
            session.namespace_selection(),
            session.capabilities(),
            feeds.map(|s| s.per_feed),
        );
    }
    out
}

fn stats(connector: &KubeConnector, cluster: &ClusterId) -> FeedStats {
    connector.feeds(cluster).expect("a live connection").stats()
}

/// Prints the budget counters after a step, for the PR and for a failing run's log.
fn report(step: &str, stats: &FeedStats) {
    eprintln!(
        "{step}: feeds {} (idle {}), started {}, stopped {}, restarts {}, objects {}, events {}",
        stats.feeds,
        stats.idle_feeds,
        stats.started,
        stats.stopped,
        stats.restarts,
        stats.objects,
        stats.events
    );
}

/// How many of the cluster's feeds have a subscriber (the ones the selection needs now).
fn active_feeds(stats: &FeedStats) -> usize {
    stats.feeds - stats.idle_feeds
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_contexts_connect_and_namespace_scoping_switches_feeds() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin_client = kind.admin_client().await;
    let a = TestNamespace::create(kind.context.as_str()).expect("namespace a");
    let b = TestNamespace::create(kind.context.as_str()).expect("namespace b");
    let run = a.name().to_owned();
    create_pods(&admin_client, a.name(), &run, "pod-a", A_PODS).await;
    create_pods(&admin_client, b.name(), &run, "pod-b", B_PODS).await;
    let (a_pods, b_pods) = (pods_in(&a, "pod-a", A_PODS), pods_in(&b, "pod-b", B_PODS));

    // Two contexts of the one kind cluster: the admin, and a user who reads pods in `a` only.
    let viewer_token = read_only_pod_viewer(&admin_client, a.name(), "oxi-smoke-viewer").await;
    let mut kubeconfig = kind.kubeconfig.clone();
    let viewer_context = ContextName::new(format!("oxi-smoke-viewer-{}", a.name()));
    kind.add_token_context(&mut kubeconfig, viewer_context.as_str(), &viewer_token);
    let admin = catalog_entry(&kind.context);
    let viewer = catalog_entry(&viewer_context);

    let connector = KubeConnector::new(
        kubeconfig,
        PoolConfig::default(),
        ConnectorConfig {
            budget: BudgetConfig {
                idle_grace: IDLE_GRACE,
                ..BudgetConfig::default()
            },
            ..ConnectorConfig::default()
        },
    );
    let manager = ClusterSessionManager::with_config(
        Arc::new(connector.clone()),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
        SessionManagerConfig::default(),
    );
    let namespaces = NamespaceService::new(
        manager.clone(),
        Arc::new(FakeStatePort::new()),
        Arc::new(TokioClock),
    );
    let diag = || diagnostics(&manager, &connector);

    // --- connect both contexts side by side, with no UI ---------------------------------------
    let viewer_selection = NamespaceSelection::single(a.name());
    manager.open(&admin, SessionOptions::default());
    manager.open(
        &viewer,
        SessionOptions {
            namespace_selection: viewer_selection.clone(),
            ..SessionOptions::default()
        },
    );
    tokio::join!(connect(&manager, &admin), connect(&manager, &viewer));

    let session_of = |entry: &ClusterContext| -> ClusterSession {
        manager.get(&entry.cluster).expect("open session")
    };
    let (admin_session, viewer_session) = (session_of(&admin), session_of(&viewer));
    assert_ne!(admin_session.id(), viewer_session.id(), "distinct sessions");
    assert_eq!(manager.sessions().len(), 2);
    assert!(
        admin_session.is_connected() && viewer_session.is_connected(),
        "{}",
        diag()
    );
    // Independent state: each session probed its own user's rights.
    let full =
        Capabilities::MUTATE | Capabilities::EXEC | Capabilities::LOGS | Capabilities::PORTFORWARD;
    assert!(
        admin_session.capabilities().contains(full),
        "{:?}",
        admin_session.capabilities()
    );
    assert!(
        !viewer_session.capabilities().contains(Capabilities::MUTATE),
        "{:?}",
        viewer_session.capabilities()
    );
    assert!(admin_session.namespace_selection().is_all());
    assert_eq!(viewer_session.namespace_selection(), &viewer_selection);

    // The namespace list is the cluster's own for the admin, and RBAC-forbidden for the viewer.
    let listed = namespaces
        .catalog(admin_session.id())
        .await
        .expect("admin catalog");
    assert_eq!(listed.source, NamespaceSource::Cluster);
    assert!(
        listed.contains(a.name()) && listed.contains(b.name()),
        "{listed:?}"
    );
    let refused = namespaces
        .catalog(viewer_session.id())
        .await
        .expect("viewer catalog");
    assert_eq!(refused.source, NamespaceSource::Forbidden, "{refused:?}");

    // --- feeds follow the namespace selection -------------------------------------------------
    let admin_id = admin_session.id().clone();
    let viewer_id = viewer_session.id().clone();
    let admin_feeds = ScopedPods::start(
        &manager,
        connector.feeds(&admin_id).expect("admin budget"),
        &admin_id,
        &run,
    )
    .await;
    let viewer_feeds = ScopedPods::start(
        &manager,
        connector.feeds(&viewer_id).expect("viewer budget"),
        &viewer_id,
        &run,
    )
    .await;

    // All: one cluster-wide feed yields the pods of both namespaces.
    let both: BTreeSet<String> = a_pods.union(&b_pods).cloned().collect();
    eventually(
        "All yields both namespaces",
        || admin_feeds.describe(),
        || async { admin_feeds.pods() == both },
    )
    .await;
    assert_eq!(admin_feeds.feed_keys(), vec![None], "one cluster-wide feed");
    assert_eq!(active_feeds(&stats(&connector, &admin_id)), 1);
    report("All", &stats(&connector, &admin_id));

    // {a}: the cluster-wide feed stops, a namespaced feed for `a` starts.
    namespaces
        .select(&admin_id, NamespaceSelection::single(a.name()))
        .await
        .expect("select {a}");
    eventually(
        "{a} yields only a's pods",
        || admin_feeds.describe(),
        || async { admin_feeds.pods() == a_pods },
    )
    .await;
    assert_eq!(admin_feeds.feed_keys(), vec![Some(a.name().to_owned())]);
    // Re-scoped, not accumulated: the released cluster-wide feed idles, then is torn down.
    eventually("the cluster-wide feed is torn down", diag, || async {
        let s = stats(&connector, &admin_id);
        s.feeds == 1 && active_feeds(&s) == 1
    })
    .await;

    // {b}: the old feed goes, the new one yields b's pods.
    namespaces
        .select(&admin_id, NamespaceSelection::single(b.name()))
        .await
        .expect("select {b}");
    eventually(
        "{b} yields only b's pods",
        || admin_feeds.describe(),
        || async { admin_feeds.pods() == b_pods },
    )
    .await;
    assert_eq!(admin_feeds.feed_keys(), vec![Some(b.name().to_owned())]);
    eventually("a's feed is torn down", diag, || async {
        let s = stats(&connector, &admin_id);
        s.feeds == 1 && s.stopped >= 2
    })
    .await;
    let after_b = stats(&connector, &admin_id);
    assert_eq!(
        after_b.started, 3,
        "cluster-wide, a, b: one feed started per scope"
    );

    // {a, b}: both namespaces, two namespaced feeds, b's is kept (not restarted).
    let restarts_before = stats(&connector, &admin_id).restarts;
    namespaces
        .select(
            &admin_id,
            NamespaceSelection::from_names([a.name(), b.name()]),
        )
        .await
        .expect("select {a, b}");
    eventually(
        "{a, b} yields both",
        || admin_feeds.describe(),
        || async { admin_feeds.pods() == both },
    )
    .await;
    let both_scoped = stats(&connector, &admin_id);
    assert_eq!(active_feeds(&both_scoped), 2, "{}", diag());
    report("{a, b}", &both_scoped);
    assert_eq!(
        both_scoped.restarts,
        restarts_before + 1,
        "only a's feed listed again; b's kept its feed"
    );

    // Back to All: one cluster-wide feed, the two namespaced ones are released and torn down.
    namespaces
        .select(&admin_id, NamespaceSelection::All)
        .await
        .expect("select All");
    eventually(
        "All yields both again",
        || admin_feeds.describe(),
        || async { admin_feeds.pods() == both },
    )
    .await;
    eventually("the namespaced feeds are torn down", diag, || async {
        let s = stats(&connector, &admin_id);
        s.feeds == 1 && active_feeds(&s) == 1
    })
    .await;

    report("All again", &stats(&connector, &admin_id));

    // --- the other session never noticed ------------------------------------------------------
    assert_eq!(session_of(&viewer).namespace_selection(), &viewer_selection);
    assert_eq!(viewer_feeds.pods(), a_pods);
    let viewer_stats = stats(&connector, &viewer_id);
    assert_eq!(
        (
            viewer_stats.feeds,
            viewer_stats.started,
            viewer_stats.stopped
        ),
        (1, 1, 0)
    );

    // Disconnecting one cluster tears its connection down and leaves the other `Ready`.
    drop(admin_feeds);
    manager.disconnect(&admin_id).expect("disconnect admin");
    assert_eq!(session_of(&admin).phase(), SessionPhase::Disconnected);
    assert!(
        connector.feeds(&admin_id).is_none(),
        "its budget went with the connection"
    );
    assert_eq!(
        session_of(&viewer).phase(),
        SessionPhase::Ready,
        "{}",
        diag()
    );
    assert_eq!(viewer_feeds.pods(), a_pods);
}
