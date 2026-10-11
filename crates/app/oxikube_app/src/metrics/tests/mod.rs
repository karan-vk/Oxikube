//! Tests of node and namespace aggregation, hot reload of the GPU list, and invariants.

mod allocation;
mod reload;

use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::quantity::Quantity;
use serde_json::{Value, json};

pub(super) fn cluster(name: &str) -> ClusterId {
    ClusterId::new("test", &ContextName::new(name))
}

pub(super) fn q(text: &str) -> Quantity {
    Quantity::parse(text).unwrap()
}

/// A pod in `ns` on `node` (when `Some`) in `phase`, with one container asking `requests` and
/// bounded by `limits` (either may be `null`).
pub(super) fn pod(
    name: &str,
    ns: &str,
    node: Option<&str>,
    phase: &str,
    requests: Value,
    limits: Value,
) -> Resource {
    let mut resources = serde_json::Map::new();
    if !requests.is_null() {
        resources.insert("requests".into(), requests);
    }
    if !limits.is_null() {
        resources.insert("limits".into(), limits);
    }
    let mut spec = json!({"containers": [{"name": "c", "image": "i", "resources": resources}]});
    if let Some(node) = node {
        spec["nodeName"] = json!(node);
    }
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": ns},
        "spec": spec, "status": {"phase": phase},
    }))
    .unwrap()
}

pub(super) fn node(name: &str, allocatable: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Node",
        "metadata": {"name": name},
        "status": {"allocatable": allocatable},
    }))
    .unwrap()
}
