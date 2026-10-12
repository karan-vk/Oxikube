//! Tests of the allocation views (E13-S11): the node and namespace rows over a fixture whose
//! numbers are derived by hand, the usage fetch against the fake metrics port, and a cross-check
//! with the S02 sums.

mod namespace;
mod node;
mod row;
mod usage;

use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::quantity::Quantity;
use serde_json::{Value, json};

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::new("kind-oxikube"))
}

pub(super) fn q(text: &str) -> Quantity {
    Quantity::parse(text).unwrap()
}

/// A pod in `ns` on `node` in `phase` with one container asking `requests` and bounded by
/// `limits` (`null` for none).
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

pub(super) fn quota(name: &str, spec: Value, status: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ResourceQuota",
        "metadata": {"name": name, "namespace": "shop"},
        "spec": spec, "status": status,
    }))
    .unwrap()
}

/// Node `n1` (4 CPU, 8Gi, 110 pods, 100Gi ephemeral, 4 NVIDIA GPUs) and the pods of `shop` and
/// `batch` on it:
///
/// | pod | ns | phase | cpu req / lim | memory req / lim | other |
/// |---|---|---|---|---|---|
/// | a | shop | Running | 500m / 1 | 256Mi / 512Mi | ephemeral 1Gi / 2Gi |
/// | b | shop | Running | 250m / - | 128Mi / - | |
/// | c | shop | Pending | 1 / 3 | 1Gi / 2Gi | |
/// | e | batch | Running | 1 / 1 | 1Gi / 1Gi | 2 GPUs |
/// | z | shop | Running | - / - | - / - | |
/// | f | shop | Succeeded | 8 / 8 | 8Gi / 8Gi | holds nothing: not counted |
/// | g | shop | Pending, unplaced | 100m / - | 64Mi / - | counts for `shop` only |
///
/// On `n1` (a, b, c, e, z): CPU requests 2750m (68 % of 4000m), limits 5000m (125 %); memory
/// requests 2432Mi, limits 3584Mi; 5 pods. In `shop` (a, b, c, z, g): CPU requests 1850m, limits
/// 4 (1 + 3); memory requests 1472Mi (256 + 128 + 1024 + 64), limits 2560Mi; 5 pods.
pub(super) fn fixture() -> (Resource, Vec<Resource>) {
    let n1 = node(
        "n1",
        json!({"cpu": "4", "memory": "8Gi", "pods": "110",
               "ephemeral-storage": "100Gi", "nvidia.com/gpu": "4"}),
    );
    let pods = vec![
        pod(
            "a",
            "shop",
            Some("n1"),
            "Running",
            json!({"cpu": "500m", "memory": "256Mi", "ephemeral-storage": "1Gi"}),
            json!({"cpu": "1", "memory": "512Mi", "ephemeral-storage": "2Gi"}),
        ),
        pod(
            "b",
            "shop",
            Some("n1"),
            "Running",
            json!({"cpu": "250m", "memory": "128Mi"}),
            json!(null),
        ),
        pod(
            "c",
            "shop",
            Some("n1"),
            "Pending",
            json!({"cpu": "1", "memory": "1Gi"}),
            json!({"cpu": "3", "memory": "2Gi"}),
        ),
        pod(
            "e",
            "batch",
            Some("n1"),
            "Running",
            json!({"cpu": "1", "memory": "1Gi", "nvidia.com/gpu": "2"}),
            json!({"cpu": "1", "memory": "1Gi", "nvidia.com/gpu": "2"}),
        ),
        pod("z", "shop", Some("n1"), "Running", json!(null), json!(null)),
        pod(
            "f",
            "shop",
            Some("n1"),
            "Succeeded",
            json!({"cpu": "8", "memory": "8Gi"}),
            json!({"cpu": "8", "memory": "8Gi"}),
        ),
        pod(
            "g",
            "shop",
            None,
            "Pending",
            json!({"cpu": "100m", "memory": "64Mi"}),
            json!(null),
        ),
    ];
    (n1, pods)
}
