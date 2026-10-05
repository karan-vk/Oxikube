//! Kind integration for E04-S11: `MetricsPort` against the real metrics-server of the kind
//! cluster, absence against a real 404 and a restricted account against a real 403.
//! Needs `cargo xtask kind-up` (installs metrics-server) and `OXIKUBE_TEST_CONTEXT`; skips
//! cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::task::{Context, Poll};

use http::Request;
use jiff::Timestamp;
use k8s_openapi::api::rbac::v1::PolicyRule;
use kube::client::{Body, ClientBuilder};
use kube::config::KubeConfigOptions;
use kube::{Client, Config};
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::view::NodeSummary;
use oxikube_kube::KubeMetrics;
use oxikube_ports::{ListOptions, MetricsOutcome, MetricsPort, ResourceReader};
use oxikube_testkit::integration::TestNamespace;
use tower::{Layer, Service};

use common::resources::adapter;
use common::{DEADLINE, Kind, TestServiceAccount, wait_until};

fn cluster(kind: &Kind) -> ClusterId {
    ClusterId::new("kind-test", &kind.context)
}

/// Polls until metrics-server has scraped something: its first samples arrive up to a minute
/// after it starts, and node and pod caches fill independently.
async fn samples_when_ready<F, Fut>(what: &str, mut call: F) -> Vec<MetricsSample>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = oxikube_domain::OxiResult<MetricsOutcome>>,
{
    wait_until(what, DEADLINE * 4, || {
        let next = call();
        async move {
            match next.await.expect("metrics call") {
                MetricsOutcome::Available(samples)
                    if !samples.is_empty() && samples.iter().all(|s| !s.is_partial()) =>
                {
                    Some(samples)
                }
                MetricsOutcome::Available(_) => None,
                MetricsOutcome::Unavailable(reason) => {
                    panic!("metrics-server should be installed on the kind cluster: {reason:?}")
                }
            }
        }
    })
    .await
}

fn pod_name(sample: &MetricsSample) -> (&str, &str) {
    match &sample.subject {
        MetricsSubject::Pod { pod } => (pod.namespace().unwrap_or_default(), &pod.name),
        other => panic!("expected a pod sample, got {other:?}"),
    }
}

#[tokio::test]
async fn node_metrics_report_usage_and_utilisation_for_every_node() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let client = kind.admin_client().await;
    let port = KubeMetrics::new((*client).clone(), cluster(&kind));
    let id = cluster(&kind);

    let samples = samples_when_ready("node metrics", || port.node_metrics(&id)).await;

    let nodes = adapter(&client)
        .list(&Gvk::new("", "v1", "Node"), None, &ListOptions::default())
        .await
        .expect("list nodes");
    assert_eq!(samples.len(), nodes.items.len(), "one sample per node");
    for node in &nodes.items {
        let summary = NodeSummary::from_resource(node).expect("node summary");
        let sample = samples
            .iter()
            .find(
                |s| matches!(&s.subject, MetricsSubject::Node { node } if **node == *summary.name),
            )
            .unwrap_or_else(|| panic!("a sample for node {}", summary.name));

        // Real usage: positive, and a believable share of what the node can allocate.
        let cpu = sample.cpu_percent_of(&summary.allocatable_cpu.expect("allocatable cpu"));
        let memory =
            sample.memory_percent_of(&summary.allocatable_memory.expect("allocatable memory"));
        assert!(
            matches!(cpu, Some(p) if p > 0.0 && p <= 100.0),
            "cpu {cpu:?}"
        );
        assert!(
            matches!(memory, Some(p) if p > 0.0 && p <= 100.0),
            "memory {memory:?}"
        );
        // The sample is recent and says how long it averages over.
        let age = Timestamp::now().duration_since(sample.ts);
        assert!(
            age.as_secs() < 600 && age.as_secs() > -60,
            "sample age {age:?}"
        );
        assert!(
            sample.window.is_some_and(|w| w.as_secs() > 0),
            "window {:?}",
            sample.window
        );
    }
}

#[tokio::test]
async fn pod_metrics_cover_kube_system_in_one_namespace_and_in_all() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let client = kind.admin_client().await;
    let port = KubeMetrics::new((*client).clone(), cluster(&kind));
    let id = cluster(&kind);

    let kube_system = NamespaceSelection::from_names(["kube-system"]);
    let scoped = samples_when_ready("kube-system pod metrics", || {
        port.pod_metrics(&id, &kube_system)
    })
    .await;
    assert!(scoped.iter().all(|s| pod_name(s).0 == "kube-system"));
    let server = scoped
        .iter()
        .find(|s| pod_name(s).1.starts_with("metrics-server-"))
        .expect("a sample for the metrics-server pod itself");
    assert!(server.cpu.value().is_some() && server.memory_bytes().is_some_and(|b| b > 0));

    let all = samples_when_ready("all-namespace pod metrics", || {
        port.pod_metrics(&id, &NamespaceSelection::All)
    })
    .await;
    assert!(all.len() >= scoped.len());
    for sample in &scoped {
        assert!(
            all.iter().any(|s| s.subject == sample.subject),
            "{:?} is in the namespaced list but not the all-namespaces one",
            pod_name(sample)
        );
    }
}

/// Rewrites the API group of every request, so a real API server answers 404 for it exactly as
/// it does for a cluster without metrics-server.
#[derive(Clone)]
struct RewriteGroup<S> {
    inner: S,
}

struct RewriteGroupLayer;

impl<S> Layer<S> for RewriteGroupLayer {
    type Service = RewriteGroup<S>;
    fn layer(&self, inner: S) -> Self::Service {
        RewriteGroup { inner }
    }
}

impl<S> Service<Request<Body>> for RewriteGroup<S>
where
    S: Service<Request<Body>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let rewritten = req.uri().to_string().replace(
            "/apis/metrics.k8s.io/",
            "/apis/metrics.absent.oxikube.test/",
        );
        *req.uri_mut() = rewritten.parse().expect("rewritten uri");
        self.inner.call(req)
    }
}

async fn client_without_metrics_group(kind: &Kind) -> Client {
    let options = KubeConfigOptions {
        context: Some(kind.context.to_string()),
        ..KubeConfigOptions::default()
    };
    let config = Config::from_custom_kubeconfig(kind.kubeconfig.clone(), &options)
        .await
        .expect("kind client config");
    ClientBuilder::try_from(config)
        .expect("client builder")
        .with_layer(&RewriteGroupLayer)
        .build()
}

#[tokio::test]
async fn a_cluster_without_the_metrics_group_is_unavailable_not_an_error() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let port = KubeMetrics::new(client_without_metrics_group(&kind).await, cluster(&kind));
    let id = cluster(&kind);

    let nodes = port
        .node_metrics(&id)
        .await
        .expect("absence is not an error");
    assert_eq!(
        nodes,
        MetricsOutcome::Unavailable(MissingReason::NotInstalled)
    );
    let pods = port
        .pod_metrics(&id, &NamespaceSelection::All)
        .await
        .expect("absence is not an error");
    assert_eq!(
        pods,
        MetricsOutcome::Unavailable(MissingReason::NotInstalled)
    );
}

#[tokio::test]
async fn an_account_without_rights_on_metrics_gets_forbidden() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    // Rights on pods only: nothing on metrics.k8s.io.
    let pods_only = PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec!["pods".into()]),
        verbs: vec!["get".into(), "list".into()],
        ..PolicyRule::default()
    };
    let account =
        TestServiceAccount::create(&admin, ns.name(), "oxi-no-metrics", vec![pods_only]).await;
    let context = ContextName::from("oxi-no-metrics");
    let client = kind
        .pool(kind.with_token_context(context.as_str(), &account.token))
        .get(&context)
        .await
        .expect("restricted client");
    let port = KubeMetrics::new((*client).clone(), cluster(&kind));
    let id = cluster(&kind);

    let err = wait_until("RBAC to deny metrics", DEADLINE, || async {
        port.node_metrics(&id).await.err()
    })
    .await;
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    let namespaces = NamespaceSelection::from_names([ns.name()]);
    let err = port
        .pod_metrics(&id, &namespaces)
        .await
        .expect_err("forbidden");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}
