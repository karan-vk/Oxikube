//! Network: Services, Endpoints and EndpointSlices (E12-S13), Ingresses, IngressClasses,
//! NetworkPolicies and the Gateway API (E12-S14).
//!
//! The Service selects the seeded Pod; the explicit Endpoints and EndpointSlice carry names no
//! controller manages; the IngressClass is never the default (a class-less Ingress elsewhere
//! keeps whatever default the cluster has); the NetworkPolicy selects a label no pod has.

use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Scope;
use serde_json::{Value, json};

use super::{Api, KindCase, Seed};

/// A kind of the Network section with the generic actions only.
const fn network(
    group: &'static str,
    version: &'static str,
    kind: &'static str,
    plural: &'static str,
    scope: Scope,
    columns: &'static [&'static str],
    seed: fn(&Seed<'_>) -> Value,
) -> KindCase {
    KindCase {
        section: "network",
        plural,
        group,
        version,
        kind,
        scope,
        api: Api::BuiltIn,
        columns,
        actions: &[],
        panel: true,
        seed,
    }
}

/// A Gateway API kind: optional, its CRDs installed or not.
const fn gateway_api(
    version: &'static str,
    kind: &'static str,
    plural: &'static str,
    scope: Scope,
    columns: &'static [&'static str],
    seed: fn(&Seed<'_>) -> Value,
) -> KindCase {
    KindCase {
        api: Api::Optional,
        ..network(GATEWAY, version, kind, plural, scope, columns, seed)
    }
}

const NAMESPACED: Scope = Scope::Namespaced;
const CLUSTER: Scope = Scope::Cluster;
const GATEWAY: &str = "gateway.networking.k8s.io";
const ROUTE_COLUMNS: &[&str] = &[
    "Name",
    "Namespace",
    "Hostnames",
    "Parents",
    "Rules",
    "Accepted",
    "Age",
];

pub(super) const CASES: &[KindCase] = &[
    KindCase {
        // `service::OpenInBrowser` is on the bus since the port-forward manager (E15-S03); the start
        // dialog's `pod::PortForward` / `portforward::Start` rows arrive with E15-S04.
        actions: &[
            CommandId::WORKLOAD_VIEW_LOGS,
            CommandId::SERVICE_SHOW_PODS,
            CommandId::SERVICE_OPEN_IN_BROWSER,
            CommandId::PORTFORWARD_START,
        ],
        ..network(
            "",
            "v1",
            "Service",
            "services",
            NAMESPACED,
            &[
                "Name",
                "Namespace",
                "Type",
                "Cluster IP",
                "External IP",
                "Ports",
                "Selector",
                "Status",
                "Age",
                "Labels*",
            ],
            |_| {
                json!({"metadata": {}, "spec": {
                    "selector": {"app": "seed-pod"},
                    "ports": [{"name": "http", "port": 80, "targetPort": 80, "protocol": "TCP"}],
                }})
            },
        )
    },
    network(
        "",
        "v1",
        "Endpoints",
        "endpoints",
        NAMESPACED,
        &["Name", "Namespace", "Endpoints", "Age"],
        |_| {
            json!({"metadata": {}, "subsets": [{
                "addresses": [{"ip": "10.0.0.1"}],
                "ports": [{"name": "http", "port": 80, "protocol": "TCP"}],
            }]})
        },
    ),
    network(
        "discovery.k8s.io",
        "v1",
        "EndpointSlice",
        "endpointslices",
        NAMESPACED,
        &[
            "Name",
            "Namespace",
            "Address Type",
            "Ports",
            "Endpoints",
            "Age",
        ],
        |_| {
            json!({
                "metadata": {"labels": {"kubernetes.io/service-name": "seed-service"}},
                "addressType": "IPv4",
                "endpoints": [{"addresses": ["10.0.0.1"], "conditions": {"ready": true}}],
                "ports": [{"name": "http", "port": 80, "protocol": "TCP"}],
            })
        },
    ),
    network(
        "networking.k8s.io",
        "v1",
        "Ingress",
        "ingresses",
        NAMESPACED,
        &[
            "Name",
            "Namespace",
            "Load Balancers",
            "Rules",
            "Age",
            "Class*",
            "Hosts*",
            "Ports*",
            "Labels*",
        ],
        |_| {
            json!({"metadata": {}, "spec": {"rules": [{
                "host": "seed.example.test",
                "http": {"paths": [{"path": "/", "pathType": "Prefix", "backend": {
                    "service": {"name": "seed-service", "port": {"number": 80}},
                }}]},
            }]}})
        },
    ),
    KindCase {
        actions: &[CommandId::INGRESS_CLASS_SET_DEFAULT],
        ..network(
            "networking.k8s.io",
            "v1",
            "IngressClass",
            "ingressclasses",
            CLUSTER,
            &[
                "Name",
                "Default",
                "Controller",
                "API Group",
                "Scope",
                "Kind",
                "Age",
                "Parameters Name*",
            ],
            |_| json!({"metadata": {}, "spec": {"controller": "oxikube.test/none"}}),
        )
    },
    network(
        "networking.k8s.io",
        "v1",
        "NetworkPolicy",
        "networkpolicies",
        NAMESPACED,
        &[
            "Name",
            "Namespace",
            "Policy Types",
            "Pod Selector",
            "Age",
            "Ingress*",
            "Egress*",
        ],
        |_| {
            json!({"metadata": {}, "spec": {
                "podSelector": {"matchLabels": {"app": "seed-netpol"}},
                "policyTypes": ["Ingress"],
            }})
        },
    ),
    gateway_api(
        "v1",
        "GatewayClass",
        "gatewayclasses",
        CLUSTER,
        &["Name", "Controller", "Accepted", "Age", "Description*"],
        |_| json!({"metadata": {}, "spec": {"controllerName": "oxikube.test/none"}}),
    ),
    gateway_api(
        "v1",
        "Gateway",
        "gateways",
        NAMESPACED,
        &[
            "Name",
            "Namespace",
            "Class",
            "Addresses",
            "Listeners",
            "Programmed",
            "Age",
        ],
        |seed| {
            json!({"metadata": {}, "spec": {
                "gatewayClassName": format!("{}-gatewayclass", seed.stem),
                "listeners": [{"name": "http", "port": 80, "protocol": "HTTP"}],
            }})
        },
    ),
    gateway_api(
        "v1",
        "HTTPRoute",
        "httproutes",
        NAMESPACED,
        ROUTE_COLUMNS,
        route,
    ),
    gateway_api(
        "v1",
        "GRPCRoute",
        "grpcroutes",
        NAMESPACED,
        ROUTE_COLUMNS,
        route,
    ),
    gateway_api(
        "v1alpha2",
        "TLSRoute",
        "tlsroutes",
        NAMESPACED,
        ROUTE_COLUMNS,
        route,
    ),
];

/// A route attached to the seeded Gateway, sending to the seeded Service.
fn route(_: &Seed<'_>) -> Value {
    json!({"metadata": {}, "spec": {
        "parentRefs": [{"name": "seed-gateway"}],
        "hostnames": ["seed.example.test"],
        "rules": [{"backendRefs": [{"name": "seed-service", "port": 80}]}],
    }})
}
