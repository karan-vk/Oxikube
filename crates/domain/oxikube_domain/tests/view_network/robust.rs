//! No JSON shape under `spec` or `status` panics a Network view-model.

use oxikube_domain::view::network::{
    GatewayClassSummary, GatewaySummary, IngressClassSummary, IngressSummary, NetworkPolicySummary,
    RouteSummary,
};
use proptest::prelude::*;
use serde_json::{Value, json};

use super::object;

fn any_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        "[a-zA-Z0-9/.*:-]{0,8}".prop_map(Value::from),
    ];
    leaf.prop_recursive(4, 48, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(Value::from),
            prop::collection::hash_map(
                prop::sample::select(vec![
                    "rules",
                    "http",
                    "paths",
                    "backend",
                    "service",
                    "port",
                    "name",
                    "number",
                    "tls",
                    "hosts",
                    "ingress",
                    "egress",
                    "from",
                    "to",
                    "ports",
                    "ipBlock",
                    "podSelector",
                    "namespaceSelector",
                    "matchLabels",
                    "matchExpressions",
                    "listeners",
                    "parentRefs",
                    "backendRefs",
                    "matches",
                    "status",
                    "parents",
                    "conditions",
                    "addresses",
                    "parameters",
                    "policyTypes",
                    "type",
                    "status",
                    "kind",
                    "group",
                    "namespace",
                    "hostnames",
                    "hostname",
                ]),
                inner,
                0..6
            )
            .prop_map(|m| Value::Object(m.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())),
        ]
    })
}

proptest! {
    #[test]
    fn constructors_never_panic(spec in any_json(), status in any_json()) {
        let body = json!({"spec": spec, "status": status});
        let make = |api: &str, kind: &str| object(api, kind, Some("ns"), "x", body.clone());
        let _ = IngressSummary::from_resource(&make("networking.k8s.io/v1", "Ingress"));
        let _ = IngressClassSummary::from_resource(&make("networking.k8s.io/v1", "IngressClass"));
        let _ = NetworkPolicySummary::from_resource(&make("networking.k8s.io/v1", "NetworkPolicy"));
        let _ = GatewayClassSummary::from_resource(&make("gateway.networking.k8s.io/v1", "GatewayClass"));
        let _ = GatewaySummary::from_resource(&make("gateway.networking.k8s.io/v1", "Gateway"));
        for kind in ["HTTPRoute", "GRPCRoute", "TLSRoute"] {
            let _ = RouteSummary::from_resource(&make("gateway.networking.k8s.io/v1", kind));
        }
    }
}
