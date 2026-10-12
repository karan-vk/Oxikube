//! The agent's metrics hooks (E13-S12) over the real adapters and the cluster's metrics-server
//! (installed by `cargo xtask kind-up`): `k8s.top_pods`, `k8s.top_nodes` and `k8s.query_promql`
//! through the `ToolRegistry`, and `@metrics/pod/<ns>/<name>` through the `ContextRegistry`.
//!
//! * the kind node and a test pod are ranked with real readings, the node with its percent of
//!   allocatable, and `@metrics` quotes the pod's requests and a reading;
//! * `query_promql` on a cluster with no Prometheus is an `unavailable` result with the reason;
//! * an account that may not read `metrics.k8s.io` gets `unavailable: no permission`, from the
//!   tools and from the mention, never a zero.
//!
//! The pod lives in the test's own `oxi-test-<rand>` namespace. Nothing is installed in the cluster.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::PostParams;
use oxikube_app::context::ContextRegistry;
use oxikube_app::metrics::live::MetricsService;
use oxikube_app::metrics_provider::{MetricsProviderService, NoSink};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::store::Spawner;
use oxikube_app::tools::ToolRegistry;
use oxikube_app::tools::metrics::{MetricsAgentDeps, QUERY_PROMQL, TOP_NODES, TOP_PODS, install};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_ports::{ContextScope, DetectionResult, DetectionState, ToolContext};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{
    FakeClusterSourcePort, FakePromqlPort, FakeProviderDetectorPort, FakeQueryCataloguePort,
};
use serde_json::{Value, json};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry, read_only_pod_viewer};

/// metrics-server scrapes every 15 s and reports a new pod after one to two scrapes.
const METRICS_DEADLINE: Duration = Duration::from_secs(120);

fn tokio_spawner() -> Arc<dyn Spawner> {
    Arc::new(|task: BoxFuture<'static, ()>| {
        tokio::spawn(task);
    })
}

struct Agent {
    tools: ToolRegistry,
    contexts: ContextRegistry,
    cluster: ClusterId,
}

impl Agent {
    async fn connect(kubeconfig: kube::config::Kubeconfig, context: &ContextName) -> Self {
        let entry = catalog_entry(context);
        let sessions = ClusterSessionManager::with_config(
            Arc::new(KubeConnector::new(
                kubeconfig,
                PoolConfig::default(),
                ConnectorConfig::default(),
            )),
            Arc::new(FakeClusterSourcePort::new()),
            Arc::new(TokioClock),
            SessionManagerConfig::default(),
        );
        sessions.open(&entry, SessionOptions::default());
        sessions.connect(&entry.cluster).await.expect("connect");

        // Detection finds nothing: a kind cluster has no Prometheus.
        let detector = Arc::new(FakeProviderDetectorPort::new());
        detector
            .script()
            .detect
            .push_ok(DetectionResult::of(DetectionState::NotFound));
        let provider = MetricsProviderService::new(sessions.clone(), detector, Arc::new(NoSink));
        provider.start(tokio_spawner());
        let live = MetricsService::new(sessions.clone(), Arc::new(TokioClock));
        let deps = MetricsAgentDeps {
            sessions,
            live,
            provider: provider.clone(),
            promql: Arc::new(FakePromqlPort::new()),
            catalogue: Arc::new(FakeQueryCataloguePort::new()),
            clock: Arc::new(TokioClock),
        };
        let (tools, contexts) = (ToolRegistry::new(), ContextRegistry::new());
        install(&tools, &contexts, &deps).expect("install");
        // The verdict of detection is part of what the tools answer.
        let started = Instant::now();
        while provider
            .status(&entry.cluster)
            .unavailable_reason()
            .is_none()
        {
            assert!(started.elapsed() < Duration::from_secs(10), "detection ran");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Self {
            tools,
            contexts,
            cluster: entry.cluster,
        }
    }

    async fn call(&self, tool: &str, args: Value) -> oxikube_ports::ToolOutput {
        let ctx = ToolContext::agent().with_cluster(self.cluster.clone());
        self.tools
            .invoke(tool, args, &ctx)
            .await
            .expect("a valid call")
    }
}

fn data(output: &oxikube_ports::ToolOutput) -> &Value {
    output.structured.as_ref().expect("structured data")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nodes_pods_and_the_mention_read_real_metrics() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pod: Pod = serde_json::from_value(oxikube_testkit::integration::pods::sleeper("busy"))
        .expect("pod json");
    Api::<Pod>::namespaced(client, ns.name())
        .create(&PostParams::default(), &pod)
        .await
        .expect("create the pod");

    let agent = Agent::connect(kind.kubeconfig.clone(), &kind.context).await;

    let nodes = agent.call(TOP_NODES, json!({"limit": 5})).await;
    assert!(!nodes.is_error, "{nodes:?}");
    let node = &data(&nodes)["nodes"][0];
    assert_eq!(data(&nodes)["status"], "ok");
    assert!(node["cpu_millicores"].as_u64().is_some(), "{node}");
    assert!(node["memory_bytes"].as_u64().unwrap() > 0, "{node}");
    assert!(
        node["cpu_percent_of_allocatable"].as_f64().is_some(),
        "the node's allocatable was read: {node}"
    );

    // metrics-server reports a new pod after a scrape or two.
    let started = Instant::now();
    let pods = loop {
        let out = agent
            .call(TOP_PODS, json!({"namespace": ns.name(), "limit": 5}))
            .await;
        if data(&out)["returned"].as_u64().unwrap_or(0) > 0 {
            break out;
        }
        assert!(
            started.elapsed() < METRICS_DEADLINE,
            "timed out waiting for the test pod's metrics: {}",
            data(&out)
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    let row = &data(&pods)["pods"][0];
    assert_eq!(row["name"], "busy");
    assert_eq!(row["namespace"], ns.name());
    assert!(row["memory_bytes"].as_u64().unwrap() > 0);

    let mention = format!("@metrics/pod/{}/busy", ns.name());
    let scope = ContextScope::new().with_cluster(agent.cluster.clone());
    let blocks = agent
        .contexts
        .resolve_text(&mention, &scope)
        .await
        .expect("the mention resolves");
    let body = &blocks[0].body;
    assert!(
        body.contains(&format!("# pod: {}/busy", ns.name())),
        "{body}"
    );
    assert!(
        body.contains("# requests:"),
        "the pod's spec was read: {body}"
    );
    assert!(
        body.contains("one reading now"),
        "nobody is watching: {body}"
    );

    let unknown = format!("@metrics/pod/{}/ghost", ns.name());
    let blocks = agent.contexts.resolve_text(&unknown, &scope).await.unwrap();
    assert!(
        blocks[0].body.contains("was not found"),
        "{}",
        blocks[0].body
    );

    let promql = agent.call(QUERY_PROMQL, json!({"query": "up"})).await;
    assert!(!promql.is_error);
    assert_eq!(data(&promql)["status"], "unavailable");
    assert!(
        data(&promql)["reason"]
            .as_str()
            .unwrap()
            .contains("no Prometheus-compatible service"),
        "{}",
        data(&promql)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_account_without_metrics_access_sees_unavailable_not_zero() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let token = read_only_pod_viewer(&admin, ns.name(), "oxi-metrics-agent").await;
    let mut kubeconfig = kind.kubeconfig.clone();
    let viewer = ContextName::new(format!("oxi-metrics-agent-{}", ns.name()));
    kind.add_token_context(&mut kubeconfig, viewer.as_str(), &token);
    let agent = Agent::connect(kubeconfig, &viewer).await;

    let pods = agent.call(TOP_PODS, json!({"namespace": ns.name()})).await;
    assert!(!pods.is_error);
    assert_eq!(data(&pods)["status"], "unavailable");
    assert_eq!(
        data(&pods)["reason"],
        "no permission to read metrics.k8s.io"
    );
    assert!(data(&pods).get("pods").is_none(), "no rows, no zeroes");

    let nodes = agent.call(TOP_NODES, json!({})).await;
    assert_eq!(data(&nodes)["status"], "unavailable");
}
