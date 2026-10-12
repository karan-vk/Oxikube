//! Live metrics (E13-S03) against the real adapter and the cluster's metrics-server (installed by
//! `cargo xtask kind-up`): the `MetricsService` polls only what is watched and shown, the node's
//! and a pod's usage reach the columns' source, and an account that may not read
//! `metrics.k8s.io` is a visible `Forbidden`, never a zero.
//!
//! The pod lives in the test's own `oxi-test-<rand>` namespace; the session's namespace selection
//! is that namespace, so the poll never lists another suite's pods. Nothing is installed in the
//! cluster.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::PostParams;
use oxikube_app::columns::{ColumnProvider, PodColumns};
use oxikube_app::metrics::live::{MetricsKind, MetricsService, MetricsState};
use oxikube_app::nodes::NodeUsageSource;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::store::{Spawner, StoreObject};
use oxikube_domain::ids::ContextName;
use oxikube_domain::metrics::MissingReason;
use oxikube_domain::session::NamespaceSelection;
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::FakeClusterSourcePort;
use oxikube_testkit::integration::TestNamespace;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry, read_only_pod_viewer};

/// metrics-server scrapes every 15 s and reports a new pod after one to two scrapes.
const METRICS_DEADLINE: Duration = Duration::from_secs(120);

fn tokio_spawner() -> Arc<dyn Spawner> {
    Arc::new(|task: BoxFuture<'static, ()>| {
        tokio::spawn(task);
    })
}

async fn until(what: &str, check: impl Fn() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(
            started.elapsed() < METRICS_DEADLINE,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn manager(kubeconfig: kube::config::Kubeconfig) -> ClusterSessionManager {
    ClusterSessionManager::with_config(
        Arc::new(KubeConnector::new(
            kubeconfig,
            PoolConfig::default(),
            ConnectorConfig::default(),
        )),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
        SessionManagerConfig::default(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nodes_and_a_pod_reach_the_columns_from_metrics_server() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pod: Pod = serde_json::from_value(oxikube_testkit::integration::pods::sleeper("busy"))
        .expect("pod json");
    Api::<Pod>::namespaced(client.clone(), ns.name())
        .create(&PostParams::default(), &pod)
        .await
        .expect("create the pod");

    let entry = catalog_entry(&kind.context);
    let cluster = entry.cluster.clone();
    let sessions = manager(kind.kubeconfig.clone());
    sessions.open(&entry, SessionOptions::default());
    sessions.connect(&cluster).await.expect("connect");
    sessions
        .set_namespace_selection(&cluster, NamespaceSelection::single(ns.name()))
        .expect("select the test namespace");

    let service = MetricsService::new(sessions.clone(), Arc::new(TokioClock));
    service.start(tokio_spawner());
    let usage = service.usage(&cluster);

    // Watched but not shown: nothing is asked.
    let nodes = service.watch(&cluster, MetricsKind::Nodes);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!service.is_polling(&cluster));
    assert_eq!(
        service.state(&cluster, MetricsKind::Nodes),
        MetricsState::Loading
    );

    // Shown: the node's usage arrives.
    let mut window = service.visibility();
    window.show(Some(cluster.clone()));
    until("node metrics", || {
        service.state(&cluster, MetricsKind::Nodes).is_available()
    })
    .await;
    let (node_count, _) = usage.counts();
    assert!(node_count >= 1, "the kind node has a sample");
    let node_name = Api::<k8s_openapi::api::core::v1::Node>::all(client.clone())
        .list(&Default::default())
        .await
        .expect("list nodes")
        .items
        .into_iter()
        .find_map(|node| node.metadata.name)
        .expect("a node");
    let node_usage = usage.usage(&node_name);
    assert!(
        node_usage.is_some_and(|u| u.cpu_millis > 0 && u.memory_bytes > 0),
        "{node_name}: {node_usage:?}"
    );
    assert_eq!(
        usage.counts().1,
        0,
        "pods are not polled while nobody shows them"
    );

    // A pods table: the test pod's CPU and memory, through the pod columns.
    let _pods = service.watch(&cluster, MetricsKind::Pods);
    until("the test pod's metrics", || {
        usage.pod(ns.name(), "busy").is_some()
    })
    .await;
    let live: Pod = Api::<Pod>::namespaced(client.clone(), ns.name())
        .get("busy")
        .await
        .expect("get the pod");
    let resource = oxikube_domain::Resource::from_json(serde_json::to_value(&live).unwrap())
        .expect("a resource");
    let columns = Arc::new(PodColumns::new()).in_cluster(Some(usage.clone()));
    let object = StoreObject::Resource(resource);
    let memory = columns.cell(
        &object,
        &oxikube_app::ColumnId::new("memory"),
        jiff::Timestamp::now(),
    );
    assert!(
        memory.display().ends_with("Mi") || memory.display().ends_with("Ki"),
        "{memory:?}"
    );

    // Nobody looks any more: the poller stops.
    drop(nodes);
    drop(_pods);
    assert!(!service.is_polling(&cluster));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_account_without_metrics_access_sees_forbidden_not_zero() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    // May read pods in the test namespace and nothing else: no `metrics.k8s.io`.
    let token = read_only_pod_viewer(&admin, ns.name(), "oxi-metrics-viewer").await;
    let mut kubeconfig = kind.kubeconfig.clone();
    let viewer = ContextName::new(format!("oxi-metrics-viewer-{}", ns.name()));
    kind.add_token_context(&mut kubeconfig, viewer.as_str(), &token);
    let entry = catalog_entry(&viewer);
    let sessions = manager(kubeconfig);
    sessions.open(&entry, SessionOptions::default());
    sessions.connect(&entry.cluster).await.expect("connect");
    sessions
        .set_namespace_selection(&entry.cluster, NamespaceSelection::single(ns.name()))
        .expect("select");

    let service = MetricsService::new(sessions.clone(), Arc::new(TokioClock));
    service.start(tokio_spawner());
    let mut window = service.visibility();
    window.show(Some(entry.cluster.clone()));
    let _pods = service.watch(&entry.cluster, MetricsKind::Pods);
    until("the verdict", || {
        !matches!(
            service.state(&entry.cluster, MetricsKind::Pods),
            MetricsState::Idle | MetricsState::Loading
        )
    })
    .await;
    let state = service.state(&entry.cluster, MetricsKind::Pods);
    assert_eq!(state, MetricsState::Unavailable(MissingReason::Forbidden));
    assert_eq!(
        state.unavailable_text().as_deref(),
        Some("metrics unavailable: no permission to read metrics.k8s.io")
    );
    assert_eq!(service.usage(&entry.cluster).counts(), (0, 0));
}
