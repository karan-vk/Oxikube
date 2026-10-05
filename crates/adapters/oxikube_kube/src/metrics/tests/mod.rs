//! Tests for `MetricsPort` on `metrics.k8s.io`: fixtures shared by the files below.

mod convert;
mod duration;
mod fallback;
mod port;

use oxikube_domain::ids::{ClusterId, ContextName};
use serde_json::{Value, json};

use super::KubeMetrics;
use crate::fake_api::FakeApi;

pub(super) const NODES: &str = "/apis/metrics.k8s.io/v1beta1/nodes";
pub(super) const ALL_PODS: &str = "/apis/metrics.k8s.io/v1beta1/pods";

pub(super) fn ns_pods(ns: &str) -> String {
    format!("/apis/metrics.k8s.io/v1beta1/namespaces/{ns}/pods")
}

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::new("kind-oxikube"))
}

pub(super) fn metrics(api: &FakeApi) -> KubeMetrics {
    KubeMetrics::new(api.client(), cluster())
}

/// A `NodeMetrics` list item, shaped like metrics-server's (no `kind` on items).
pub(super) fn node_item(name: &str, cpu: &str, memory: &str) -> Value {
    json!({
        "metadata": {"name": name, "creationTimestamp": "2026-10-03T12:00:05Z"},
        "timestamp": "2026-10-03T12:00:00Z",
        "window": "14.982s",
        "usage": {"cpu": cpu, "memory": memory},
    })
}

pub(super) fn node_list(items: Vec<Value>) -> Value {
    json!({"kind": "NodeMetricsList", "apiVersion": "metrics.k8s.io/v1beta1", "metadata": {}, "items": items})
}

/// A `PodMetrics` list item with one `(cpu, memory)` pair per container.
pub(super) fn pod_item(ns: &str, name: &str, containers: &[(&str, &str)]) -> Value {
    let containers: Vec<Value> = containers
        .iter()
        .enumerate()
        .map(|(i, (cpu, memory))| {
            json!({"name": format!("c{i}"), "usage": {"cpu": cpu, "memory": memory}})
        })
        .collect();
    json!({
        "metadata": {"name": name, "namespace": ns, "creationTimestamp": "2026-10-03T12:00:05Z"},
        "timestamp": "2026-10-03T12:00:01Z",
        "window": "15.5s",
        "containers": containers,
    })
}

pub(super) fn pod_list(items: Vec<Value>) -> Value {
    json!({"kind": "PodMetricsList", "apiVersion": "metrics.k8s.io/v1beta1", "metadata": {}, "items": items})
}
