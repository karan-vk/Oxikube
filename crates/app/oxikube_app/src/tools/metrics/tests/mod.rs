//! Tests of the metrics tools and `@metrics` over fake sessions, a scripted metrics port, a
//! scripted PromQL port and a fake detector. Tokio test runtime; every wait is on a condition with
//! a deadline, never a sleep before an assertion.

mod inventory;
mod promql;
mod schema;
mod top;

use std::sync::Arc;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::MetricsProviderKind;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::metrics::{ContainerUsage, MetricsSample, MetricsSubject, Reading};
use oxikube_domain::{OxiResult, Resource};
use oxikube_ports::{
    ClusterContext, ClusterPrefs, ClusterPrefsTable, DetectedProvider, DetectionResult,
    DetectionState, MetricsOutcome, PrometheusOverride, ProviderTarget, ServicePort, ServiceRef,
    SourceId, ToolContext, ToolOutput,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeMetricsPort,
    FakePromqlPort, FakeProviderDetectorPort, FakeQueryCataloguePort,
};
use serde_json::{Value, json};

use super::{MetricsAgentDeps, install};
use crate::context::ContextRegistry;
use crate::metrics::live::MetricsService;
use crate::metrics_provider::{MetricsProviderService, NoSink};
use crate::session::ClusterSessionManager;
use crate::store::Spawner;
use crate::tools::ToolRegistry;

pub(crate) const MI: u64 = 1024 * 1024;
pub(crate) const MILLI: u64 = 1_000_000;

pub(crate) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("dev"))
}

pub(crate) fn t0() -> Timestamp {
    "2026-01-01T00:00:00Z".parse().expect("a timestamp")
}

pub(crate) fn at(seconds: i64) -> Timestamp {
    t0() + SignedDuration::from_secs(seconds)
}

pub(crate) fn pod_sample(
    ns: &str,
    name: &str,
    ts: Timestamp,
    millis: u64,
    mib: u64,
) -> MetricsSample {
    let pod = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), ns, name);
    let mut sample = MetricsSample::new(
        MetricsSubject::Pod { pod },
        ts,
        None,
        millis * MILLI,
        mib * MI,
    );
    sample.containers = vec![ContainerUsage {
        name: "app".into(),
        cpu: Reading::Value(millis * MILLI),
        memory: Reading::Value(mib * MI),
    }];
    sample
}

pub(crate) fn node_sample(name: &str, ts: Timestamp, millis: u64, mib: u64) -> MetricsSample {
    MetricsSample::new(
        MetricsSubject::Node { node: name.into() },
        ts,
        None,
        millis * MILLI,
        mib * MI,
    )
}

pub(crate) fn available(samples: Vec<MetricsSample>) -> MetricsOutcome {
    MetricsOutcome::Available(samples)
}

pub(crate) fn pod_object(ns: &str, name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": ns},
        "spec": {"containers": [{
            "name": "app",
            "resources": {
                "requests": {"cpu": "100m", "memory": "128Mi"},
                "limits": {"cpu": "500m", "memory": "256Mi"}
            }
        }]}
    }))
    .expect("a pod")
}

pub(crate) fn node_object(name: &str, cpu: &str, memory: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Node",
        "metadata": {"name": name},
        "status": {"allocatable": {"cpu": cpu, "memory": memory, "pods": "110"}}
    }))
    .expect("a node")
}

fn tokio_spawner() -> Arc<dyn Spawner> {
    Arc::new(|task: futures::future::BoxFuture<'static, ()>| {
        tokio::spawn(task);
    })
}

/// A connected fake cluster with every service the metrics tools read through.
pub(crate) struct Harness {
    pub(crate) sessions: ClusterSessionManager,
    pub(crate) connector: Arc<FakeClusterConnectorPort>,
    pub(crate) live: MetricsService,
    pub(crate) provider: MetricsProviderService,
    pub(crate) detector: Arc<FakeProviderDetectorPort>,
    pub(crate) promql: Arc<FakePromqlPort>,
    pub(crate) catalogue: Arc<FakeQueryCataloguePort>,
    pub(crate) tools: ToolRegistry,
    pub(crate) contexts: ContextRegistry,
}

impl Harness {
    /// A cluster `dev`, not connected yet.
    pub(crate) fn new() -> Self {
        Self::with_prefs(ClusterPrefsTable::default())
    }

    pub(crate) fn with_prefs(prefs: ClusterPrefsTable) -> Self {
        let context = ContextName::new("dev");
        let entry = ClusterContext::new(cluster(), context, SourceId("kubeconfig".into()));
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
        let sessions = ClusterSessionManager::new(
            connector.clone(),
            source,
            Arc::new(FakeClockPort::default()),
        );
        sessions.set_prefs_table(prefs);
        let clock = Arc::new(FakeClockPort::new(t0()));
        let live = MetricsService::new(sessions.clone(), clock.clone());
        let detector = Arc::new(FakeProviderDetectorPort::new());
        let provider =
            MetricsProviderService::new(sessions.clone(), detector.clone(), Arc::new(NoSink));
        provider.start(tokio_spawner());
        let promql = Arc::new(FakePromqlPort::new());
        let catalogue = Arc::new(FakeQueryCataloguePort::new());
        let deps = MetricsAgentDeps {
            sessions: sessions.clone(),
            live: live.clone(),
            provider: provider.clone(),
            promql: promql.clone(),
            catalogue: catalogue.clone(),
            clock: clock,
        };
        let tools = ToolRegistry::new();
        let contexts = ContextRegistry::new();
        install(&tools, &contexts, &deps).expect("install");
        Self {
            sessions,
            connector,
            live,
            provider,
            detector,
            promql,
            catalogue,
            tools,
            contexts,
        }
    }

    pub(crate) async fn connect(&self) {
        self.sessions.connect(&cluster()).await.expect("connect");
    }

    /// Connects, with detection finding `kind`.
    pub(crate) async fn connect_with_provider(&self, kind: MetricsProviderKind) {
        self.detector
            .script()
            .detect
            .push_ok(DetectionResult::of(DetectionState::Found(
                DetectedProvider {
                    kind,
                    target: ProviderTarget::Service(ServiceRef::new(
                        "monitoring",
                        "prometheus-operated",
                        ServicePort::Name("web".into()),
                    )),
                    path_prefix: None,
                },
            )));
        self.connect().await;
        self.until(|h| h.provider.status(&cluster()).provider().is_some())
            .await;
    }

    /// Connects, with detection finding nothing.
    pub(crate) async fn connect_without_provider(&self) {
        self.detector
            .script()
            .detect
            .push_ok(DetectionResult::of(DetectionState::NotFound));
        self.connect().await;
        self.until(|h| {
            matches!(
                h.provider.status(&cluster()).state,
                crate::metrics_provider::ProviderState::Detection(DetectionState::NotFound)
            )
        })
        .await;
    }

    pub(crate) async fn until(&self, done: impl Fn(&Self) -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !done(self) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the condition held in time");
    }

    pub(crate) fn metrics(&self) -> Arc<FakeMetricsPort> {
        self.connector.ports_for(&cluster()).metrics
    }

    pub(crate) fn objects(&self) -> Arc<oxikube_testkit::FakeResourcePort> {
        self.connector.ports_for(&cluster()).resources
    }

    /// Calls `tool` for the only connected cluster.
    pub(crate) async fn call(&self, tool: &str, args: Value) -> OxiResult<ToolOutput> {
        self.tools.invoke(tool, args, &ToolContext::agent()).await
    }
}

/// The structured part of a successful, non-error answer.
pub(crate) fn payload(output: &ToolOutput) -> &Value {
    assert!(!output.is_error, "a tool error: {:?}", output.content);
    output.structured.as_ref().expect("structured data")
}

/// The text the model reads.
pub(crate) fn text(output: &ToolOutput) -> String {
    output
        .content
        .iter()
        .filter_map(|part| part.as_text())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Settings that turn history metrics off for the cluster.
pub(crate) fn prometheus_off() -> ClusterPrefsTable {
    ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
        cluster(),
        ClusterPrefs {
            prometheus: Some(PrometheusOverride {
                enabled: false,
                ..PrometheusOverride::default()
            }),
            ..ClusterPrefs::default()
        },
    )
}
