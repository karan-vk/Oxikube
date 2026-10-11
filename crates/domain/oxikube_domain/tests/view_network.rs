//! Fixture-driven tests of the Network section's view-models (E12-S14): Ingress, IngressClass,
//! NetworkPolicy and the Gateway API kinds, from raw JSON.

#[path = "view_network/gateway.rs"]
mod gateway;
#[path = "view_network/ingress.rs"]
mod ingress;
#[path = "view_network/policy.rs"]
mod policy;
#[path = "view_network/robust.rs"]
mod robust;

use oxikube_domain::Resource;
use serde_json::{Value, json};

/// A namespaced (or, with `namespace` `None`, cluster-scoped) object with `body` merged in.
pub fn object(
    api_version: &str,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
    body: Value,
) -> Resource {
    let mut metadata = json!({"name": name, "creationTimestamp": "2026-09-01T00:00:00Z"});
    if let Some(ns) = namespace {
        metadata["namespace"] = json!(ns);
    }
    let mut value = json!({"apiVersion": api_version, "kind": kind, "metadata": metadata});
    for (key, part) in body.as_object().cloned().unwrap_or_default() {
        value[key] = part;
    }
    Resource::from_json(value).expect("a resource")
}
