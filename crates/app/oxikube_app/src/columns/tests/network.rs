//! The Network section's columns (E12-S14): Ingress, IngressClass, NetworkPolicy and the Gateway
//! API kinds, fixture JSON in, cell texts and tones out.

use oxikube_domain::Capabilities;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::fixtures as fx;
use serde_json::json;

use super::{cell, check, cluster, namespaced};
use crate::columns::{ColumnProvider, CoreColumns, Tone};
use crate::store::{FeedKind, FeedPolicy};

const GATEWAY_API: &str = "gateway.networking.k8s.io/v1";

fn titles(group: &str, kind: &str) -> Vec<(String, bool)> {
    CoreColumns::new()
        .columns(&Gvk::new(group, "v1", kind), Capabilities::empty())
        .iter()
        .map(|c| (c.title.to_string(), c.wide))
        .collect()
}

#[test]
fn ingress_lists_load_balancers_and_rules_by_default() {
    let visible: Vec<String> = titles("networking.k8s.io", "Ingress")
        .into_iter()
        .filter(|(_, wide)| !wide)
        .map(|(title, _)| title)
        .collect();
    assert_eq!(
        visible,
        ["Name", "Namespace", "Load Balancers", "Rules", "Age"]
    );
    check(
        &fx::ingress(),
        &[
            ("address", "203.0.113.10"),
            ("rules", "web.example.com/ → web:80, */health → web:80"),
            ("class", "nginx"),
            ("hosts", "web.example.com,*"),
            ("ports", "80, 443"),
        ],
    );
}

#[test]
fn ingress_class_has_the_default_badge_and_its_parameters() {
    let visible: Vec<String> = titles("networking.k8s.io", "IngressClass")
        .into_iter()
        .filter(|(_, wide)| !wide)
        .map(|(title, _)| title)
        .collect();
    assert_eq!(
        visible,
        [
            "Name",
            "Default",
            "Controller",
            "API Group",
            "Scope",
            "Kind",
            "Age"
        ]
    );
    let plain = cluster(
        "networking.k8s.io/v1",
        "IngressClass",
        json!({"spec": {"controller": "k8s.io/ingress-nginx"}}),
    );
    check(
        &plain,
        &[
            ("default", ""),
            ("parameters-kind", ""),
            ("parameters-scope", ""),
        ],
    );
    assert_eq!(cell(&plain, "default").tone(), Tone::Neutral);

    let mut default = cluster(
        "networking.k8s.io/v1",
        "IngressClass",
        json!({"spec": {
            "controller": "k8s.io/ingress-nginx",
            "parameters": {"apiGroup": "k8s.example.com", "kind": "IngressParameters", "name": "p"}
        }}),
    );
    default.edit_json(|v| {
        v["metadata"]["annotations"] =
            json!({"ingressclass.kubernetes.io/is-default-class": "true"});
    });
    // `edit_json` leaves the parsed metadata as it was: read the edited object again.
    let default = oxikube_domain::Resource::from_json(default.to_value()).unwrap();
    check(
        &default,
        &[
            ("default", "Default"),
            ("controller", "k8s.io/ingress-nginx"),
            ("parameters-api-group", "k8s.example.com"),
            ("parameters-scope", "Cluster"),
            ("parameters-kind", "IngressParameters"),
            ("parameters-name", "p"),
        ],
    );
    assert_eq!(cell(&default, "default").tone(), Tone::Ok);
}

#[test]
fn network_policy_shows_types_selector_and_what_it_does_to_each_direction() {
    let visible: Vec<String> = titles("networking.k8s.io", "NetworkPolicy")
        .into_iter()
        .filter(|(_, wide)| !wide)
        .map(|(title, _)| title)
        .collect();
    assert_eq!(
        visible,
        ["Name", "Namespace", "Policy Types", "Pod Selector", "Age"]
    );

    let deny_all = namespaced(
        "networking.k8s.io/v1",
        "NetworkPolicy",
        json!({"spec": {"podSelector": {}, "policyTypes": ["Ingress", "Egress"]}}),
    );
    check(
        &deny_all,
        &[
            ("pod-selector", "all pods"),
            ("policy-types", "Ingress,Egress"),
            ("ingress", "Deny all"),
            ("egress", "Deny all"),
        ],
    );
    assert_eq!(cell(&deny_all, "ingress").tone(), Tone::Warn);

    let rules = namespaced(
        "networking.k8s.io/v1",
        "NetworkPolicy",
        json!({"spec": {
            "podSelector": {"matchLabels": {"app": "db"}},
            "ingress": [{"from": [{"podSelector": {}}]}, {"ports": [{"port": 5432}]}],
            "egress": [{}]
        }}),
    );
    // The API defaults the types to Ingress plus Egress when there are egress rules.
    check(
        &rules,
        &[
            ("pod-selector", "app=db"),
            ("policy-types", "Ingress,Egress"),
            ("ingress", "2 rules"),
            ("egress", "Allow all"),
        ],
    );
    let ingress_only = namespaced(
        "networking.k8s.io/v1",
        "NetworkPolicy",
        json!({"spec": {"podSelector": {}, "ingress": [{"ports": [{"port": 80}]}]}}),
    );
    check(&ingress_only, &[("ingress", "1 rule"), ("egress", "")]);
}

#[test]
fn gateway_api_kinds_have_columns_and_a_full_feed() {
    let policy = FeedPolicy::new();
    let provider = CoreColumns::new();
    for kind in [
        "GatewayClass",
        "Gateway",
        "HTTPRoute",
        "GRPCRoute",
        "TLSRoute",
    ] {
        let gvk = Gvk::new("gateway.networking.k8s.io", "v1", kind);
        assert!(provider.knows(&gvk), "{kind}");
        assert_eq!(policy.plan(&gvk).kind, FeedKind::Full, "{kind}");
        // Any served version: the lookup ignores it.
        assert!(provider.knows(&Gvk::new("gateway.networking.k8s.io", "v1alpha2", kind)));
    }
}

#[test]
fn gateway_class_and_gateway_show_their_conditions() {
    let class = cluster(
        GATEWAY_API,
        "GatewayClass",
        json!({
            "spec": {"controllerName": "example.com/gw", "description": "Example"},
            "status": {"conditions": [{"type": "Accepted", "status": "True", "reason": "Accepted"}]}
        }),
    );
    check(
        &class,
        &[
            ("controller", "example.com/gw"),
            ("accepted", "True"),
            ("description", "Example"),
        ],
    );
    assert_eq!(cell(&class, "accepted").tone(), Tone::Ok);

    let fresh = cluster(
        GATEWAY_API,
        "GatewayClass",
        json!({"spec": {"controllerName": "x"}}),
    );
    assert_eq!(cell(&fresh, "accepted").display(), "Unknown");
    assert_eq!(cell(&fresh, "accepted").tone(), Tone::Warn);

    let gateway = namespaced(
        GATEWAY_API,
        "Gateway",
        json!({
            "spec": {"gatewayClassName": "envoy", "listeners": [
                {"name": "http", "protocol": "HTTP", "port": 80},
                {"name": "https", "protocol": "HTTPS", "port": 443, "hostname": "api.example.com"}
            ]},
            "status": {
                "addresses": [{"type": "IPAddress", "value": "203.0.113.7"}],
                "conditions": [{"type": "Programmed", "status": "False", "reason": "AddressNotAssigned"}]
            }
        }),
    );
    check(
        &gateway,
        &[
            ("class", "envoy"),
            ("addresses", "203.0.113.7"),
            ("listeners", "HTTP :80, HTTPS :443 api.example.com"),
            ("programmed", "False"),
        ],
    );
    assert_eq!(cell(&gateway, "programmed").tone(), Tone::Error);
}

#[test]
fn routes_show_hostnames_parents_rules_and_acceptance() {
    let route = namespaced(
        GATEWAY_API,
        "HTTPRoute",
        json!({
            "spec": {
                "hostnames": ["shop.example.com"],
                "parentRefs": [{"name": "edge", "namespace": "infra"}],
                "rules": [{"backendRefs": [{"name": "web", "port": 80}]}, {"backendRefs": [{"name": "api", "port": 80}]}]
            },
            "status": {"parents": [{
                "parentRef": {"name": "edge", "namespace": "infra"},
                "controllerName": "example.com/gw",
                "conditions": [{"type": "Accepted", "status": "False", "reason": "NotAllowedByListeners"}]
            }]}
        }),
    );
    check(
        &route,
        &[
            ("hostnames", "shop.example.com"),
            ("parents", "infra/edge"),
            ("rules", "2"),
            ("accepted", "False"),
        ],
    );
    assert_eq!(cell(&route, "accepted").tone(), Tone::Error);
    // The same columns serve the other route kinds, at whatever version the cluster serves.
    let tls = namespaced(
        "gateway.networking.k8s.io/v1alpha2",
        "TLSRoute",
        json!({"spec": {"parentRefs": [{"name": "edge"}], "rules": [{"backendRefs": [{"name": "t", "port": 1}]}]}}),
    );
    check(
        &tls,
        &[
            ("hostnames", "*"),
            ("parents", "demo/edge"),
            ("rules", "1"),
            ("accepted", "Unknown"),
        ],
    );
}
