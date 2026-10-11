//! The Network panels in the detail drawer (E12-S14): an Ingress with its rules table and load
//! balancer, a NetworkPolicy with default-deny egress and an IP block with an exception, and an
//! HTTPRoute with a parent that refused it and a backend that does not resolve, in the dark and
//! the light theme.
//!
//! The ages are read against a fixed "now", so the pictures do not change with the clock.

use oxikube_domain::Resource;
use oxikube_resources_ui::detail::DetailTab;
use serde_json::{Value, json};

use super::check;
use super::detail::{HEIGHT, WIDTH, render_of};

fn object(api_version: &str, kind: &str, name: &str, body: Value) -> Resource {
    let mut value = json!({
        "apiVersion": api_version, "kind": kind,
        "metadata": {"name": name, "namespace": "shop", "resourceVersion": "9", "uid": "uid-1",
                     "creationTimestamp": "2026-01-01T00:00:00Z"}
    });
    for (key, part) in body.as_object().cloned().unwrap_or_default() {
        value[key] = part;
    }
    Resource::from_json(value).expect("a resource")
}

fn ingress() -> Resource {
    object(
        "networking.k8s.io/v1",
        "Ingress",
        "web",
        json!({
            "spec": {
                "ingressClassName": "nginx",
                "tls": [{"hosts": ["shop.example.com"], "secretName": "shop-tls"}],
                "rules": [
                    {"host": "shop.example.com", "http": {"paths": [
                        {"path": "/api", "pathType": "Prefix",
                         "backend": {"service": {"name": "api", "port": {"number": 8080}}}},
                        {"path": "/", "pathType": "Prefix",
                         "backend": {"service": {"name": "web", "port": {"name": "http"}}}}
                    ]}},
                    {"host": "blog.example.com", "http": {"paths": [
                        {"path": "/", "pathType": "Prefix",
                         "backend": {"service": {"name": "blog", "port": {"number": 80}}}}
                    ]}}
                ]
            },
            "status": {"loadBalancer": {"ingress": [{"ip": "203.0.113.10"}]}}
        }),
    )
}

fn policy() -> Resource {
    object(
        "networking.k8s.io/v1",
        "NetworkPolicy",
        "db-access",
        json!({"spec": {
            "podSelector": {"matchLabels": {"app": "db"}},
            "policyTypes": ["Ingress", "Egress"],
            "ingress": [{
                "from": [
                    {"podSelector": {"matchLabels": {"app": "web"}}},
                    {"namespaceSelector": {"matchLabels": {"env": "prod"}}},
                    {"ipBlock": {"cidr": "10.0.0.0/8", "except": ["10.1.0.0/16"]}}
                ],
                "ports": [{"port": 5432}, {"protocol": "UDP", "port": 53}]
            }]
        }}),
    )
}

fn route() -> Resource {
    object(
        "gateway.networking.k8s.io/v1",
        "HTTPRoute",
        "web",
        json!({
            "spec": {
                "hostnames": ["shop.example.com"],
                "parentRefs": [{"name": "edge", "namespace": "infra", "sectionName": "https"}],
                "rules": [{
                    "matches": [{"path": {"type": "PathPrefix", "value": "/api"}}],
                    "backendRefs": [{"name": "api", "port": 8080, "weight": 90},
                                    {"name": "api-canary", "port": 8080, "weight": 10}]
                }]
            },
            "status": {"parents": [{
                "parentRef": {"name": "edge", "namespace": "infra", "sectionName": "https"},
                "controllerName": "example.com/gateway-controller",
                "conditions": [
                    {"type": "Accepted", "status": "False", "reason": "NotAllowedByListeners",
                     "message": "listener https does not allow routes from namespace shop"},
                    {"type": "ResolvedRefs", "status": "False", "reason": "BackendNotFound",
                     "message": "Service shop/api-canary not found"}
                ]
            }]}
        }),
    )
}

pub(crate) fn run() -> anyhow::Result<()> {
    for (name, object) in [
        ("network_ingress", ingress as fn() -> Resource),
        ("network_policy", policy),
        ("network_httproute", route),
    ] {
        check(
            &format!("{name}_dark"),
            render_of(object(), false, DetailTab::Overview)?,
            WIDTH,
            HEIGHT,
        )?;
        check(
            &format!("{name}_light"),
            render_of(object(), true, DetailTab::Overview)?,
            WIDTH,
            HEIGHT,
        )?;
    }
    Ok(())
}
