//! The synthetic clusters the windowed scenarios run against (ADR 0016): the app's real services,
//! stores and views over ports that produce a scenario's load exactly and in real time, so a run
//! needs no cluster and every run sees the same load.
//!
//! What is synthetic is only what a cluster would send: the kubeconfig contexts (testkit's
//! `FakeClusterSourcePort`), the connect (`FakeClusterConnectorPort`), and per cluster the objects
//! and their changes ([`WorldResources`] over a [`Hub`]: 10 000 pods churning 1 % every 5 s, their
//! events, a 5 MB ConfigMap), the logs ([`WorldLogs`]: 5 000 lines/s) and the describe text
//! ([`WorldDescribe`]). Everything from the session manager down to the pixels is the app's own
//! code, on the app's Tokio runtime and wall clock.
//!
//! | File | Holds |
//! |---|---|
//! | `population` | [`Population`]: the pods, the churn tick (`load-pods --churn`'s) |
//! | `hub` | [`Hub`]: live pods and events, fanned out to every watch; the churn loop |
//! | `resources` | [`WorldResources`]: the `ResourcePort` (hub for pods and events, fixed objects for the rest) |
//! | `logs` | [`WorldLogs`] and [`line`]: the mixed log at 5 000 lines/s |
//! | `describe` | [`WorldDescribe`]: `kubectl describe`-shaped text |
//! | `fixed` | the fixed objects: namespaces, nodes, workloads, services, ConfigMaps |
//! | `metrics` | [`WorldMetrics`]: the synthetic metrics-server (a cluster with `metrics`) |
//! | `prometheus` | [`WorldPromql`], [`WorldDetector`]: the synthetic Prometheus (a cluster with `prometheus`) |

mod describe;
mod fixed;
mod hub;
pub mod logs;
mod metrics;
mod population;
mod prometheus;
mod resources;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_describe::DescribePreference;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{
    ClockPort, ClusterConnection, ClusterConnectorPort, ClusterContext, ClusterSource,
    ConnectRequest, SourceId, SourceKind,
};
use oxikube_testkit::{
    FakeClusterConnectorPort, FakeClusterSourcePort, FakeFsPort, FakeResourcePort,
    FakeSecretStorePort, FakeStatePort,
};
use tokio::runtime::Handle;

use crate::app_state::{AppPorts, ClusterAdapters, PrometheusPorts};
use crate::kube_ports::{SystemClock, WatchBudgets};

pub use describe::WorldDescribe;
pub use fixed::{BIG_CONFIG_MAP, BIG_CONFIG_MAP_BYTES};
pub use hub::{Hub, Stream};
pub use logs::{WorldLogs, line};
pub use metrics::WorldMetrics;
pub use population::{Population, namespace_name};
pub use prometheus::{WorldDetector, WorldPromql};
pub use resources::WorldResources;

/// The source every synthetic context is listed under.
const SOURCE: &str = "perf-kubeconfig";
/// Events recorded about the large ConfigMap.
const BIG_CONFIG_MAP_EVENTS: usize = 40;
/// How often `load-pods --churn` recycles 1 % of the pods.
pub const CHURN_EVERY: Duration = Duration::from_secs(5);

/// One synthetic cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterSpec {
    /// Its kubeconfig context name.
    pub context: String,
    /// Pods, spread over `namespaces`.
    pub pods: usize,
    /// Namespaces.
    pub namespaces: usize,
    /// Nodes the running pods are spread over.
    pub nodes: usize,
    /// Recycle 1 % of the pods every [`CHURN_EVERY`] once connected.
    pub churn: bool,
    /// The cluster serves live metrics (a metrics-server): the overview's usage bars and top
    /// consumers have numbers (E13-S09-F1).
    pub metrics: bool,
    /// The cluster serves a Prometheus-compatible endpoint: the overview's history graphs have an
    /// hour of samples (E13-S09-F1). The endpoint is the app's, not the cluster's: the world's
    /// Prometheus answers for every cluster that asks for one.
    pub prometheus: bool,
}

impl ClusterSpec {
    /// `pods` pods in 8 namespaces under `context`, churning.
    pub fn churning(context: &str, pods: usize) -> Self {
        Self {
            context: context.to_owned(),
            pods,
            namespaces: 8,
            nodes: fixed::NODES,
            churn: true,
            metrics: false,
            prometheus: false,
        }
    }

    /// The same over `nodes` nodes.
    #[must_use]
    pub fn with_nodes(self, nodes: usize) -> Self {
        Self {
            nodes: nodes.max(1),
            ..self
        }
    }

    /// The same without churn.
    pub fn still(context: &str, pods: usize) -> Self {
        Self {
            churn: false,
            ..Self::churning(context, pods)
        }
    }

    /// The same, with a metrics-server (live CPU and memory).
    #[must_use]
    pub fn with_metrics(self) -> Self {
        Self {
            metrics: true,
            ..self
        }
    }

    /// The same, with a Prometheus-compatible endpoint (the history graphs).
    #[must_use]
    pub fn with_prometheus(self) -> Self {
        Self {
            prometheus: true,
            ..self
        }
    }

    /// The id the catalog gives the context.
    pub fn cluster_id(&self) -> ClusterId {
        cluster_id(&self.context)
    }
}

/// The id of synthetic context `context`.
pub fn cluster_id(context: &str) -> ClusterId {
    ClusterId::new(SOURCE, &ContextName::new(context))
}

/// What a scenario's world holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorldSpec {
    /// The clusters that can be connected, in catalog order.
    pub clusters: Vec<ClusterSpec>,
    /// Further contexts the catalog lists that are never connected (the catalog scenario's 50).
    pub listed_only: usize,
}

struct Cluster {
    hub: Arc<Hub>,
    churn: bool,
    churning: OnceLock<()>,
    resources: Arc<WorldResources>,
    /// The cluster's metrics-server, when it has one.
    metrics: Option<Arc<WorldMetrics>>,
}

/// The connector: testkit's fake connect (health reporter, guard), with each cluster's
/// synthetic ports in the bundle it returns.
struct WorldConnector {
    inner: FakeClusterConnectorPort,
    clusters: HashMap<ClusterId, Cluster>,
    /// Where the churn and the log streams run; without one (tests on the deterministic
    /// runtime) nothing churns and logs are their tail only.
    runtime: Option<Handle>,
}

#[async_trait]
impl ClusterConnectorPort for WorldConnector {
    async fn connect(&self, request: ConnectRequest) -> OxiResult<ClusterConnection> {
        let id = request.cluster.clone();
        let mut connection = self.inner.connect(request).await?;
        if let Some(cluster) = self.clusters.get(&id) {
            if let Some(runtime) = &self.runtime
                && cluster.churn
                && cluster.churning.set(()).is_ok()
            {
                cluster.hub.churn(CHURN_EVERY, runtime);
            }
            connection.ports.resources = cluster.resources.clone();
            connection.ports.logs = Arc::new(WorldLogs::new(self.runtime.clone()));
            connection.ports.describe = Arc::new(WorldDescribe::new(cluster.resources.clone()));
            if let Some(metrics) = &cluster.metrics {
                connection.ports.metrics = metrics.clone();
            }
        }
        Ok(connection)
    }
}

/// The app's ports over the world `spec` describes, on the wall clock, its streams on `runtime`.
/// See the [module docs](self).
pub fn app_ports(spec: &WorldSpec, runtime: Handle) -> AppPorts {
    let clock = Arc::new(SystemClock::new(runtime.clone()));
    ports_with(spec, clock, Some(runtime))
}

/// [`app_ports`] on `clock`, its streams on `runtime` when there is one (tests pass none and a
/// fake clock: nothing then runs off GPUI's executors).
pub fn ports_with(
    spec: &WorldSpec,
    clock: Arc<dyn ClockPort>,
    runtime: Option<Handle>,
) -> AppPorts {
    let now = Timestamp::now();
    let source = SourceId(SOURCE.to_owned());
    let inner = FakeClusterConnectorPort::new();
    let mut clusters = HashMap::new();
    let mut contexts = Vec::new();
    for spec in &spec.clusters {
        let id = spec.cluster_id();
        let ports = inner.ports_for(&id);
        ports.discovery.set_kinds(fixed::kinds());
        let objects = fixed::objects(spec.namespaces, spec.nodes, now);
        let fixed = Arc::new(FakeResourcePort::new().with_objects(objects));
        let events = fixed::big_config_map_events(BIG_CONFIG_MAP_EVENTS, now);
        let population = Population::new(spec.pods, spec.namespaces, now).with_nodes(spec.nodes);
        let hub = Hub::new(population, events);
        let resources = Arc::new(WorldResources::new(hub.clone(), fixed));
        clusters.insert(
            id.clone(),
            Cluster {
                hub,
                churn: spec.churn,
                churning: OnceLock::new(),
                resources,
                metrics: spec
                    .metrics
                    .then(|| Arc::new(WorldMetrics::new(spec.nodes, spec.pods, spec.namespaces))),
            },
        );
        contexts.push(context(&spec.context, &source));
    }
    for i in 0..spec.listed_only {
        contexts.push(context(&format!("perf-listed-{i:02}"), &source));
    }
    let catalog = FakeClusterSourcePort::new()
        .with_sources([ClusterSource {
            id: source,
            kind: SourceKind::KubeconfigFile,
            label: "Synthetic perf clusters".to_owned(),
            path: None,
        }])
        .with_contexts(contexts);
    let connector = WorldConnector {
        inner,
        clusters,
        runtime,
    };
    let adapters = ClusterAdapters {
        source: Arc::new(catalog),
        connector: Arc::new(connector),
        budgets: WatchBudgets::disabled(),
        clock,
        fs: Arc::new(FakeFsPort::new()),
        kubeconfigs_dir: ClusterAdapters::TEST_KUBECONFIGS_DIR.into(),
        describe: DescribePreference::default(),
        kubeconfig_origins: Default::default(),
    };
    let mut ports = AppPorts::new(Arc::new(FakeStatePort::new()), adapters)
        .with_secrets(Arc::new(FakeSecretStorePort::new()));
    if let Some(prometheus) = prometheus_ports(spec) {
        ports = ports.with_prometheus(prometheus);
    }
    ports
}

/// The synthetic Prometheus, when a cluster of `spec` asks for one: the endpoint answers for
/// every cluster, so its scale is the sum of the nodes and pods of the clusters that asked.
fn prometheus_ports(spec: &WorldSpec) -> Option<PrometheusPorts> {
    let clusters: Vec<&ClusterSpec> = spec.clusters.iter().filter(|c| c.prometheus).collect();
    if clusters.is_empty() {
        return None;
    }
    let nodes = clusters.iter().map(|c| c.nodes).sum();
    let pods = clusters.iter().map(|c| c.pods).sum();
    Some(PrometheusPorts {
        promql: Arc::new(WorldPromql::new(nodes, pods)),
        detector: Arc::new(WorldDetector),
    })
}

fn context(name: &str, source: &SourceId) -> ClusterContext {
    ClusterContext {
        server: Some(format!("https://{name}.perf.invalid:6443")),
        default_namespace: Some("default".to_owned()),
        cluster_name: Some(name.to_owned()),
        user: Some(format!("{name}-admin")),
        ..ClusterContext::new(cluster_id(name), ContextName::new(name), source.clone())
    }
}
