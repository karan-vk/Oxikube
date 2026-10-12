//! Tests of the utilisation maths: unit cases, an oracle model, and property tests.

mod oracle;
mod percent;
mod pod;
mod props;
mod quota;

use serde_json::{Value, json};

use crate::quantity::Quantity;
use crate::resource::Resource;

/// A core Pod with `spec`, scheduled on `node` (when `Some`) and in `phase`.
pub(super) fn pod(
    name: &str,
    ns: &str,
    node: Option<&str>,
    phase: &str,
    mut spec: Value,
) -> Resource {
    if let Some(node) = node {
        spec["nodeName"] = json!(node);
    }
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": name, "namespace": ns},
        "spec": spec,
        "status": {"phase": phase},
    }))
    .unwrap()
}

/// A container with the given `requests` / `limits` maps (either may be `null`).
pub(super) fn container(name: &str, requests: Value, limits: Value) -> Value {
    let mut resources = serde_json::Map::new();
    if !requests.is_null() {
        resources.insert("requests".into(), requests);
    }
    if !limits.is_null() {
        resources.insert("limits".into(), limits);
    }
    json!({"name": name, "image": "img", "resources": resources})
}

/// A restartable init container (a sidecar).
pub(super) fn sidecar(name: &str, requests: Value, limits: Value) -> Value {
    let mut c = container(name, requests, limits);
    c["restartPolicy"] = json!("Always");
    c
}

/// A Node with `status.allocatable`.
pub(super) fn node(name: &str, allocatable: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Node",
        "metadata": {"name": name},
        "status": {"allocatable": allocatable},
    }))
    .unwrap()
}

pub(super) fn q(text: &str) -> Quantity {
    Quantity::parse(text).unwrap()
}
