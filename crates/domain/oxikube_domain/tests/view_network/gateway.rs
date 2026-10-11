//! Gateway API view-models from raw JSON.

use oxikube_domain::ViewError;
use oxikube_domain::view::ConditionStatus;
use oxikube_domain::view::network::{
    GATEWAY_API_GROUP, GatewayClassSummary, GatewaySummary, RouteKind, RouteSummary,
    is_gateway_network_kind,
};
use serde_json::json;

use super::object;

fn gateway_class() -> GatewayClassSummary {
    GatewayClassSummary::from_resource(&object(
        "gateway.networking.k8s.io/v1",
        "GatewayClass",
        None,
        "envoy",
        json!({
            "spec": {"controllerName": "gateway.envoyproxy.io/controller", "description": "Envoy",
                     "parametersRef": {"group": "gateway.envoyproxy.io", "kind": "EnvoyProxy", "name": "cfg", "namespace": "envoy"}},
            "status": {"conditions": [
                {"type": "Accepted", "status": "True", "reason": "Accepted", "message": "valid"}
            ]}
        }),
    ))
    .unwrap()
}

#[test]
fn a_gateway_class_has_its_controller_and_accepted_status() {
    let class = gateway_class();
    assert_eq!(
        class.controller.as_deref(),
        Some("gateway.envoyproxy.io/controller")
    );
    assert_eq!(class.accepted(), ConditionStatus::True);
    assert_eq!(class.conditions[0].reason.as_deref(), Some("Accepted"));
    let parameters = class.parameters.expect("parameters");
    assert_eq!(&*parameters.kind, "EnvoyProxy");
    assert_eq!(parameters.label(), "envoy/cfg");
    let unreported = GatewayClassSummary::from_resource(&object(
        "gateway.networking.k8s.io/v1",
        "GatewayClass",
        None,
        "new",
        json!({"spec": {"controllerName": "x"}}),
    ))
    .unwrap();
    assert_eq!(unreported.accepted(), ConditionStatus::Unknown);
}

fn gateway() -> GatewaySummary {
    GatewaySummary::from_resource(&object(
        "gateway.networking.k8s.io/v1",
        "Gateway",
        Some("infra"),
        "edge",
        json!({
            "spec": {
                "gatewayClassName": "envoy",
                "listeners": [
                    {"name": "http", "protocol": "HTTP", "port": 80,
                     "allowedRoutes": {"namespaces": {"from": "All"}}},
                    {"name": "https", "protocol": "HTTPS", "port": 443, "hostname": "api.example.com",
                     "tls": {"certificateRefs": [{"name": "api-tls"}]},
                     "allowedRoutes": {"namespaces": {"from": "Selector"}, "kinds": [{"kind": "HTTPRoute"}]}},
                    {"name": "pass", "protocol": "TLS", "port": 8443, "tls": {"mode": "Passthrough"}}
                ]
            },
            "status": {
                "addresses": [{"type": "IPAddress", "value": "203.0.113.7"}, {"type": "Hostname", "value": "lb.example.com"}],
                "conditions": [
                    {"type": "Accepted", "status": "True", "reason": "Accepted"},
                    {"type": "Programmed", "status": "False", "reason": "AddressNotAssigned", "message": "waiting for the LB"}
                ],
                "listeners": [
                    {"name": "https", "attachedRoutes": 3, "conditions": [
                        {"type": "ResolvedRefs", "status": "False", "reason": "InvalidCertificateRef", "message": "no such secret"}
                    ]},
                    {"name": "http", "attachedRoutes": 1, "conditions": [{"type": "Programmed", "status": "True", "reason": "Programmed"}]}
                ]
            }
        }),
    ))
    .unwrap()
}

#[test]
fn a_gateway_joins_each_listener_with_its_status_by_name() {
    let g = gateway();
    assert_eq!(g.class.as_deref(), Some("envoy"));
    assert_eq!(g.accepted(), ConditionStatus::True);
    assert_eq!(g.programmed(), ConditionStatus::False);
    assert_eq!(
        g.conditions[1].message.as_deref(),
        Some("waiting for the LB")
    );
    assert_eq!(g.addresses_text(), "203.0.113.7,lb.example.com");
    assert!(!g.addresses_requested);
    assert_eq!(
        g.listeners_text(),
        "HTTP :80, HTTPS :443 api.example.com, TLS :8443"
    );

    let [http, https, pass] = &g.listeners[..] else {
        panic!("three listeners")
    };
    assert_eq!(http.attached_routes, Some(1));
    assert_eq!(http.allowed_namespaces, "All namespaces");
    assert_eq!(http.tls_mode, None);
    assert_eq!(https.attached_routes, Some(3));
    assert_eq!(https.tls_mode.as_deref(), Some("Terminate"));
    assert_eq!(https.certificates, ["api-tls"]);
    assert_eq!(https.allowed_namespaces, "Selected namespaces");
    assert_eq!(https.allowed_kinds, ["HTTPRoute"]);
    assert_eq!(&*https.conditions[0].kind, "ResolvedRefs");
    assert_eq!(https.conditions[0].status, ConditionStatus::False);
    assert_eq!(
        https.conditions[0].reason.as_deref(),
        Some("InvalidCertificateRef")
    );
    assert_eq!(pass.tls_mode.as_deref(), Some("Passthrough"));
    assert_eq!(pass.attached_routes, None, "no status entry yet");
    assert_eq!(pass.allowed_namespaces, "Same namespace");

    let class = g.class_ref().expect("a class");
    assert_eq!(
        (&*class.group, &*class.kind, class.namespace.as_deref()),
        (GATEWAY_API_GROUP, "GatewayClass", None)
    );
    assert_eq!(&*class.name, "envoy");
}

#[test]
fn requested_addresses_are_shown_as_requested_until_assigned() {
    let g = GatewaySummary::from_resource(&object(
        "gateway.networking.k8s.io/v1",
        "Gateway",
        Some("a"),
        "g",
        json!({"spec": {"gatewayClassName": "c", "addresses": [{"value": "198.51.100.9"}], "listeners": []}}),
    ))
    .unwrap();
    assert_eq!(g.addresses_text(), "198.51.100.9");
    assert!(g.addresses_requested);
    assert_eq!(g.addresses[0].kind, "IPAddress");
    assert_eq!(g.programmed(), ConditionStatus::Unknown);
}

fn http_route() -> RouteSummary {
    RouteSummary::from_resource(&object(
        "gateway.networking.k8s.io/v1",
        "HTTPRoute",
        Some("shop"),
        "web",
        json!({
            "spec": {
                "hostnames": ["shop.example.com", "*.shop.example.com"],
                "parentRefs": [
                    {"name": "edge", "namespace": "infra", "sectionName": "https"},
                    {"name": "other"}
                ],
                "rules": [
                    {"matches": [
                        {"path": {"type": "PathPrefix", "value": "/api"}, "headers": [{"name": "x-env", "value": "canary"}], "method": "GET"},
                        {"path": {"type": "Exact", "value": "/healthz"}}
                    ],
                     "filters": [{"type": "RequestHeaderModifier"}, {"type": "URLRewrite"}],
                     "backendRefs": [{"name": "api", "port": 8080, "weight": 90}, {"name": "api-canary", "port": 8080, "weight": 10, "namespace": "canary"}]},
                    {"backendRefs": [{"name": "assets", "kind": "Bucket", "group": "k8s.example.com"}]}
                ]
            },
            "status": {"parents": [
                {"parentRef": {"name": "edge", "namespace": "infra", "sectionName": "https"},
                 "controllerName": "gateway.envoyproxy.io/controller",
                 "conditions": [
                     {"type": "Accepted", "status": "True", "reason": "Accepted"},
                     {"type": "ResolvedRefs", "status": "False", "reason": "BackendNotFound", "message": "Service shop/api-canary not found"}
                 ]},
                {"parentRef": {"name": "other", "group": "gateway.networking.k8s.io", "kind": "Gateway"},
                 "controllerName": "example.com/other",
                 "conditions": [{"type": "Accepted", "status": "False", "reason": "NotAllowedByListeners", "message": "namespace not allowed"}]}
            ]}
        }),
    ))
    .unwrap()
}

#[test]
fn an_http_route_has_hostnames_rules_and_parents_with_their_status() {
    let r = http_route();
    assert_eq!(r.kind, RouteKind::Http);
    assert_eq!(r.hostnames_text(), "shop.example.com,*.shop.example.com");

    let [edge, other] = &r.parents[..] else {
        panic!("two parents")
    };
    assert_eq!(edge.label(), "infra/edge · https");
    assert_eq!(edge.accepted(), ConditionStatus::True);
    assert_eq!(
        edge.controller.as_deref(),
        Some("gateway.envoyproxy.io/controller")
    );
    assert_eq!(
        edge.conditions[1].reason.as_deref(),
        Some("BackendNotFound")
    );
    // The parent without a namespace is in the route's, and the status matched by identity.
    assert_eq!(other.label(), "shop/other");
    assert_eq!(other.accepted(), ConditionStatus::False);
    assert_eq!(
        other.conditions[0].message.as_deref(),
        Some("namespace not allowed")
    );
    assert_eq!(&*edge.target.kind, "Gateway");
    assert_eq!(&*edge.target.group, GATEWAY_API_GROUP);

    assert_eq!(r.accepted(), ConditionStatus::False, "any refusal shows");
    assert_eq!(r.parents_text(), "infra/edge · https, shop/other");

    let first = &r.rules[0];
    assert_eq!(
        first.matches,
        [
            "PathPrefix /api & method GET & header x-env=canary",
            "Exact /healthz"
        ]
    );
    assert_eq!(first.filters, ["RequestHeaderModifier", "URLRewrite"]);
    let backends: Vec<String> = first.backends.iter().map(|b| b.label()).collect();
    assert_eq!(
        backends,
        ["api:8080 (weight 90)", "api-canary:8080 (weight 10)"]
    );
    assert_eq!(
        first.backends[0].target.label(),
        "shop/api",
        "defaults to the route's namespace"
    );
    assert_eq!(first.backends[1].target.label(), "canary/api-canary");
    let second = &r.rules[1];
    assert!(second.matches.is_empty());
    assert_eq!(second.backends[0].label(), "Bucket/assets");
    assert_eq!(&*second.backends[0].target.group, "k8s.example.com");
}

#[test]
fn a_refusal_shows_whatever_the_order_of_the_parents_and_whoever_has_not_reported() {
    // "silent" has no status at all; a status for a reference the spec lacks is a parent too.
    let route = |parents: &[&str]| {
        let refs: Vec<_> = parents.iter().map(|name| json!({"name": name})).collect();
        let status: Vec<_> = parents
            .iter()
            .filter(|name| **name != "silent")
            .map(|name| {
                let verdict = if *name == "refusing" { "False" } else { "True" };
                json!({"parentRef": {"name": name}, "controllerName": "c",
                       "conditions": [{"type": "Accepted", "status": verdict}]})
            })
            .collect();
        RouteSummary::from_resource(&object(
            "gateway.networking.k8s.io/v1",
            "HTTPRoute",
            Some("a"),
            "r",
            json!({"spec": {"parentRefs": refs}, "status": {"parents": status}}),
        ))
        .unwrap()
    };
    for order in [
        ["silent", "refusing"],
        ["refusing", "silent"],
        ["accepting", "refusing"],
    ] {
        assert_eq!(
            route(&order).accepted(),
            ConditionStatus::False,
            "{order:?}"
        );
    }
    assert_eq!(
        route(&["accepting", "silent"]).accepted(),
        ConditionStatus::Unknown
    );
    assert_eq!(
        route(&["silent", "accepting"]).accepted(),
        ConditionStatus::Unknown
    );
}

#[test]
fn a_route_with_every_parent_accepting_is_accepted_and_one_nobody_reported_on_is_unknown() {
    let accepted = RouteSummary::from_resource(&object(
        "gateway.networking.k8s.io/v1",
        "HTTPRoute",
        Some("a"),
        "r",
        json!({"spec": {"parentRefs": [{"name": "g"}]},
               "status": {"parents": [{"parentRef": {"name": "g"}, "controllerName": "c",
                   "conditions": [{"type": "Accepted", "status": "True"}]}]}}),
    ))
    .unwrap();
    assert_eq!(accepted.accepted(), ConditionStatus::True);
    assert_eq!(accepted.hostnames_text(), "*");

    let fresh = RouteSummary::from_resource(&object(
        "gateway.networking.k8s.io/v1",
        "HTTPRoute",
        Some("a"),
        "r",
        json!({"spec": {"parentRefs": [{"name": "g"}]}}),
    ))
    .unwrap();
    assert_eq!(fresh.accepted(), ConditionStatus::Unknown);
    assert!(fresh.parents[0].conditions.is_empty());
}

#[test]
fn a_status_for_a_removed_parent_reference_still_shows() {
    let r = RouteSummary::from_resource(&object(
        "gateway.networking.k8s.io/v1",
        "HTTPRoute",
        Some("a"),
        "r",
        json!({"spec": {"parentRefs": [{"name": "g"}]},
               "status": {"parents": [{"parentRef": {"name": "gone"}, "controllerName": "c",
                   "conditions": [{"type": "Accepted", "status": "True"}]}]}}),
    ))
    .unwrap();
    assert_eq!(r.parents.len(), 2);
    assert_eq!(r.parents[1].target.label(), "a/gone");
}

#[test]
fn grpc_and_tls_routes_read_from_the_same_json_at_any_served_version() {
    let grpc = RouteSummary::from_resource(&object(
        "gateway.networking.k8s.io/v1",
        "GRPCRoute",
        Some("a"),
        "g",
        json!({"spec": {"hostnames": ["grpc.test"], "parentRefs": [{"name": "edge"}],
            "rules": [{"matches": [
                {"method": {"service": "helloworld.Greeter", "method": "SayHello"}},
                {"method": {"service": "helloworld.Greeter"}, "headers": [{"name": "env", "value": "x"}]}
            ], "backendRefs": [{"name": "greeter", "port": 50051}]}]}}),
    ))
    .unwrap();
    assert_eq!(grpc.kind, RouteKind::Grpc);
    assert_eq!(
        grpc.rules[0].matches,
        [
            "gRPC helloworld.Greeter/SayHello",
            "gRPC helloworld.Greeter/* & header env=x"
        ]
    );

    // TLSRoute is still experimental in many releases: served as v1alpha2 (or v1alpha3).
    for version in ["v1alpha2", "v1alpha3", "v1"] {
        let tls = RouteSummary::from_resource(&object(
            &format!("gateway.networking.k8s.io/{version}"),
            "TLSRoute",
            Some("a"),
            "t",
            json!({"spec": {"hostnames": ["secure.test"], "parentRefs": [{"name": "edge", "sectionName": "pass"}],
                "rules": [{"backendRefs": [{"name": "tls-app", "port": 8443}]}]}}),
        ))
        .unwrap();
        assert_eq!(tls.kind, RouteKind::Tls, "{version}");
        assert_eq!(tls.rules[0].backends[0].label(), "tls-app:8443");
        assert!(tls.rules[0].matches.is_empty());
    }
}

#[test]
fn route_and_gateway_constructors_refuse_other_kinds() {
    let service = object("v1", "Service", Some("a"), "s", json!({}));
    assert!(matches!(
        RouteSummary::from_resource(&service),
        Err(ViewError::WrongKind { .. })
    ));
    assert!(matches!(
        GatewaySummary::from_resource(&service),
        Err(ViewError::WrongKind { .. })
    ));
    assert!(matches!(
        GatewayClassSummary::from_resource(&service),
        Err(ViewError::WrongKind { .. })
    ));
    // A namesake in another group is not a Gateway.
    let other = object("example.com/v1", "Gateway", Some("a"), "g", json!({}));
    assert!(GatewaySummary::from_resource(&other).is_err());
}

#[test]
fn the_network_section_lists_five_gateway_kinds_and_leaves_the_rest_to_custom_resources() {
    for kind in [
        "GatewayClass",
        "Gateway",
        "HTTPRoute",
        "GRPCRoute",
        "TLSRoute",
    ] {
        assert!(is_gateway_network_kind(GATEWAY_API_GROUP, kind), "{kind}");
    }
    assert!(!is_gateway_network_kind(
        GATEWAY_API_GROUP,
        "ReferenceGrant"
    ));
    assert!(!is_gateway_network_kind("example.com", "Gateway"));
}
