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

mod describe;
mod fixed;
mod hub;
pub mod logs;
mod population;
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

use crate::app_state::{AppPorts, ClusterAdapters};
use crate::kube_ports::{SystemClock, WatchBudgets};

pub use describe::WorldDescribe;
pub use fixed::{BIG_CONFIG_MAP, BIG_CONFIG_MAP_BYTES};
pub use hub::{Hub, Stream};
pub use logs::{WorldLogs, line};
pub use population::{Population, namespace_name};
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
    /// Recycle 1 % of the pods every [`CHURN_EVERY`] once connected.
    pub churn: bool,
}

impl ClusterSpec {
    /// `pods` pods in 8 namespaces under `context`, churning.
    pub fn churning(context: &str, pods: usize) -> Self {
        Self {
            context: context.to_owned(),
            pods,
            namespaces: 8,
            churn: true,
        }
    }

    /// The same without churn.
    pub fn still(context: &str, pods: usize) -> Self {
        Self {
            churn: false,
            ..Self::churning(context, pods)
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
        let objects = fixed::objects(spec.namespaces, now);
        let fixed = Arc::new(FakeResourcePort::new().with_objects(objects));
        let events = fixed::big_config_map_events(BIG_CONFIG_MAP_EVENTS, now);
        let hub = Hub::new(Population::new(spec.pods, spec.namespaces, now), events);
        let resources = Arc::new(WorldResources::new(hub.clone(), fixed));
        clusters.insert(
            id.clone(),
            Cluster {
                hub,
                churn: spec.churn,
                churning: OnceLock::new(),
                resources,
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
    };
    AppPorts::new(Arc::new(FakeStatePort::new()), adapters)
        .with_secrets(Arc::new(FakeSecretStorePort::new()))
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
