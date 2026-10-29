//! Tests of the diff: the pure pieces, then the service over the testkit fakes.

mod pure;
mod service;

use oxikube_domain::Resource;
use serde_json::{Value, json};

/// A ConfigMap `name` in `namespace` with `data`, as a server would return it.
pub(super) fn config_map(namespace: &str, name: &str, data: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {
            "name": name, "namespace": namespace, "uid": format!("uid-{namespace}"),
            "resourceVersion": "42", "creationTimestamp": "2026-01-01T00:00:00Z",
            "managedFields": [{"manager": "kubectl"}],
            "annotations": {"kubectl.kubernetes.io/last-applied-configuration": "{}"}
        },
        "data": data
    }))
    .expect("a ConfigMap")
}

/// A Secret with plaintext-looking base64 `data`.
pub(super) fn secret(namespace: &str, name: &str, data: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": name, "namespace": namespace, "uid": "u"},
        "type": "Opaque",
        "data": data
    }))
    .expect("a Secret")
}
