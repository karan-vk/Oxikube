//! The Network section's panels, actions and discovery gating (E12-S14): the detail of an
//! Ingress, an IngressClass, a NetworkPolicy, a Gateway and an HTTPRoute, "Set as default", and
//! the Gateway API appearing only where discovery serves it.

use std::time::Duration;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_domain::access::AccessRules;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::ResourceKind;
use oxikube_domain::{OxiError, Resource};
use oxikube_testkit::{DiscoveryCall, ResourceCall, discovery_sets};
use serde_json::{Value, json};

use super::row_ids;
use crate::detail::tests::fixture::{Detail, edited, kind};
use crate::kinds::KindSpecs;
use crate::table::tests::fixture::{Fixture, cluster};

const NETWORKING: &str = "networking.k8s.io";
const GATEWAY: &str = "gateway.networking.k8s.io";
const DEFAULT: &str = "ingressclass.kubernetes.io/is-default-class";

fn object(
    api_version: &str,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
    body: Value,
) -> Resource {
    let mut metadata = json!({"name": name, "resourceVersion": "5", "uid": format!("uid-{name}"),
                              "creationTimestamp": "2026-01-01T00:00:00Z"});
    if let Some(ns) = namespace {
        metadata["namespace"] = json!(ns);
    }
    let mut value = json!({"apiVersion": api_version, "kind": kind, "metadata": metadata});
    for (key, part) in body.as_object().cloned().unwrap_or_default() {
        value[key] = part;
    }
    Resource::from_json(value).expect("a resource")
}

/// The kinds the Network tests serve.
fn served(gateway_version: &str) -> Vec<ResourceKind> {
    vec![
        kind(NETWORKING, "v1", "Ingress", "ingresses", true),
        kind(NETWORKING, "v1", "IngressClass", "ingressclasses", false),
        kind(NETWORKING, "v1", "NetworkPolicy", "networkpolicies", true),
        kind("", "v1", "Service", "services", true),
        kind(
            GATEWAY,
            gateway_version,
            "GatewayClass",
            "gatewayclasses",
            false,
        ),
        kind(GATEWAY, gateway_version, "Gateway", "gateways", true),
        kind(GATEWAY, gateway_version, "HTTPRoute", "httproutes", true),
    ]
}

/// A window with the built-in kinds registered, over a cluster serving `kinds` and holding `objects`.
fn window(cx: &mut TestAppContext, kinds: Vec<ResourceKind>, objects: Vec<Resource>) -> Detail {
    let mut d = Detail::new(cx, objects);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    d.f.ports().discovery.set_kinds(kinds);
    d
}

fn target(
    group: &str,
    version: &str,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
) -> ResourceRef {
    let gvk = Gvk::new(group, version, kind);
    match namespace {
        Some(ns) => ResourceRef::namespaced(cluster(), gvk, ns, name),
        None => ResourceRef::cluster_scoped(cluster(), gvk, name),
    }
}

fn ingress() -> Resource {
    object(
        "networking.k8s.io/v1",
        "Ingress",
        Some("shop"),
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
                    ]}}
                ]
            },
            "status": {"loadBalancer": {"ingress": [{"ip": "203.0.113.10"}]}}
        }),
    )
}

#[gpui::test]
fn an_ingress_detail_shows_its_rules_and_links_to_the_service(cx: &mut TestAppContext) {
    let mut d = window(cx, served("v1"), vec![ingress()]);
    let view = d.open(&target(NETWORKING, "v1", "Ingress", Some("shop"), "web"));
    for selector in [
        "detail-kind-panel",
        "net-ingress",
        "net-ingress-lb",
        "net-ingress-header",
        "net-ingress-rule-0",
        "net-ingress-rule-1",
        "net-ingress-backend-0",
        "net-ingress-tls",
        "detail-overview",
    ] {
        assert!(d.shown(selector), "{selector} is on screen");
    }
    // The generic rows stay under the panel.
    assert!(d.read(&view, |v| v.kind_panel().is_some()));

    d.f.dispatcher.clear();
    d.click("net-ingress-backend-0");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: target("", "v1", "Service", Some("shop"), "api")
        }],
        "the backend is a link to the Service, in the Ingress' namespace"
    );
}

#[gpui::test]
fn an_ingress_without_rules_says_so(cx: &mut TestAppContext) {
    let bare = object(
        "networking.k8s.io/v1",
        "Ingress",
        Some("shop"),
        "bare",
        json!({"spec": {}}),
    );
    let mut d = window(cx, served("v1"), vec![bare]);
    d.open(&target(NETWORKING, "v1", "Ingress", Some("shop"), "bare"));
    assert!(d.shown("net-ingress-no-rules"));
    assert!(!d.shown("net-ingress-header"));
}

fn class(name: &str, default: bool) -> Resource {
    let mut res = object(
        "networking.k8s.io/v1",
        "IngressClass",
        None,
        name,
        json!({"spec": {"controller": "k8s.io/ingress-nginx"}}),
    );
    if default {
        res = edited(res, |v| {
            v["metadata"]["annotations"] = json!({DEFAULT: "true"})
        });
    }
    res
}

#[gpui::test]
fn an_ingress_class_offers_set_as_default_until_it_is_the_default(cx: &mut TestAppContext) {
    let mut d = window(
        cx,
        served("v1"),
        vec![class("nginx", false), class("traefik", true)],
    );
    d.open(&target(NETWORKING, "v1", "IngressClass", None, "nginx"));
    assert!(d.shown("net-class-not-default"));
    assert!(!d.shown("net-class-default"));
    d.f.dispatcher.clear();
    d.click("net-class-set-default");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::IngressClassSetDefault {
            target: target(NETWORKING, "v1", "IngressClass", None, "nginx"),
            clear_previous: true,
        }],
        "the button is the command, so the guard confirms, audits and refuses it"
    );

    // The default class shows the badge and no button.
    d.open(&target(NETWORKING, "v1", "IngressClass", None, "traefik"));
    assert!(d.shown("net-class-default"));
    assert!(!d.shown("net-class-set-default"));
}

#[gpui::test]
fn on_a_read_only_cluster_the_button_is_disabled(cx: &mut TestAppContext) {
    let mut d = window(cx, served("v1"), vec![class("nginx", false)]);
    d.f.sessions
        .set_read_only(&cluster(), true)
        .expect("open session");
    d.open(&target(NETWORKING, "v1", "IngressClass", None, "nginx"));
    d.f.dispatcher.clear();
    d.click("net-class-set-default");
    assert!(
        d.f.dispatcher.sent().is_empty(),
        "a disabled button sends nothing"
    );
}

fn policy() -> Resource {
    object(
        "networking.k8s.io/v1",
        "NetworkPolicy",
        Some("db"),
        "db-access",
        json!({"spec": {
            "podSelector": {"matchLabels": {"app": "db"}},
            "policyTypes": ["Ingress", "Egress"],
            "ingress": [
                {"from": [
                    {"podSelector": {"matchLabels": {"app": "web"}}},
                    {"namespaceSelector": {"matchLabels": {"env": "prod"}}},
                    {"ipBlock": {"cidr": "10.0.0.0/8", "except": ["10.1.0.0/16"]}}
                ], "ports": [{"port": 5432}, {"protocol": "UDP", "port": 53}]}
            ]
        }}),
    )
}

#[gpui::test]
fn a_network_policy_detail_renders_rules_and_default_deny(cx: &mut TestAppContext) {
    let mut d = window(cx, served("v1"), vec![policy()]);
    d.open(&target(
        NETWORKING,
        "v1",
        "NetworkPolicy",
        Some("db"),
        "db-access",
    ));
    for selector in [
        "net-policy",
        "net-policy-selector",
        "net-policy-types",
        "net-policy-ingress-effect",
        "net-policy-ingress-rule-0",
        "net-policy-egress-effect",
        "net-policy-egress-note",
    ] {
        assert!(d.shown(selector), "{selector} is on screen");
    }
    // No egress rule at all, and Egress is a policy type: default deny. Ingress is restricted
    // to the one rule. The text itself is the domain's (pinned by its tests); here every peer,
    // the ipBlock with its exception among them, must be drawn as that text.
    for peer in [
        "net-policy-peer:IP 10.0.0.0/8 except 10.1.0.0/16",
        "net-policy-peer:pods app=web in this namespace",
        "net-policy-peer:all pods in namespaces env=prod",
    ] {
        assert!(d.shown(peer), "{peer} is on screen");
    }
    assert!(
        !d.shown("net-policy-egress-rule-0"),
        "no egress rule to draw"
    );
}

fn gateway_objects(version: &str) -> Vec<Resource> {
    let api = format!("gateway.networking.k8s.io/{version}");
    let api = api.as_str();
    vec![
        object(
            api,
            "GatewayClass",
            None,
            "envoy",
            json!({"spec": {"controllerName": "example.com/gw"},
                   "status": {"conditions": [{"type": "Accepted", "status": "True", "reason": "Accepted"}]}}),
        ),
        object(
            api,
            "Gateway",
            Some("infra"),
            "edge",
            json!({
                "spec": {"gatewayClassName": "envoy", "listeners": [
                    {"name": "http", "protocol": "HTTP", "port": 80}
                ]},
                "status": {
                    "addresses": [{"type": "IPAddress", "value": "203.0.113.7"}],
                    "conditions": [
                        {"type": "Accepted", "status": "True", "reason": "Accepted"},
                        {"type": "Programmed", "status": "False", "reason": "AddressNotAssigned",
                         "message": "waiting for the load balancer"}
                    ],
                    "listeners": [{"name": "http", "attachedRoutes": 2, "conditions": [
                        {"type": "ResolvedRefs", "status": "True", "reason": "ResolvedRefs"}
                    ]}]
                }
            }),
        ),
        object(
            api,
            "HTTPRoute",
            Some("shop"),
            "web",
            json!({
                "spec": {
                    "hostnames": ["shop.example.com"],
                    "parentRefs": [{"name": "edge", "namespace": "infra", "sectionName": "http"}],
                    "rules": [{"matches": [{"path": {"type": "PathPrefix", "value": "/"}}],
                               "backendRefs": [{"name": "web", "port": 80}]}]
                },
                "status": {"parents": [{
                    "parentRef": {"name": "edge", "namespace": "infra", "sectionName": "http"},
                    "controllerName": "example.com/gw",
                    "conditions": [
                        {"type": "Accepted", "status": "True", "reason": "Accepted"},
                        {"type": "ResolvedRefs", "status": "False", "reason": "BackendNotFound",
                         "message": "Service shop/web not found"}
                    ]
                }]}
            }),
        ),
    ]
}

#[gpui::test]
fn an_http_route_detail_shows_hostnames_parents_rules_and_their_reasons(cx: &mut TestAppContext) {
    // The cluster serves the Gateway API at v1beta1: links must use that, not a built-in version.
    let mut d = window(cx, served("v1beta1"), gateway_objects("v1beta1"));
    d.open(&target(
        GATEWAY,
        "v1beta1",
        "HTTPRoute",
        Some("shop"),
        "web",
    ));
    for selector in [
        "net-route",
        "net-route-hostnames",
        "net-route-accepted-chip",
        "net-route-parent-0",
        "net-route-parent-condition-0",
        "net-route-parent-condition-1",
        "net-route-rule-0",
        "net-route-matches",
        "net-route-backend-0",
    ] {
        assert!(d.shown(selector), "{selector} is on screen");
    }

    d.f.dispatcher.clear();
    d.click("net-route-parent-link-0");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: target(GATEWAY, "v1beta1", "Gateway", Some("infra"), "edge")
        }],
        "route to its parent Gateway, at the version discovery reports"
    );
    // The click opened the Gateway's detail in the drawer; back to the route.
    d.open(&target(
        GATEWAY,
        "v1beta1",
        "HTTPRoute",
        Some("shop"),
        "web",
    ));
    d.f.dispatcher.clear();
    d.click("net-route-backend-0");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: target("", "v1", "Service", Some("shop"), "web")
        }]
    );
}

#[gpui::test]
fn a_gateway_detail_shows_listeners_addresses_and_links_to_its_class(cx: &mut TestAppContext) {
    let mut d = window(cx, served("v1"), gateway_objects("v1"));
    d.open(&target(GATEWAY, "v1", "Gateway", Some("infra"), "edge"));
    for selector in [
        "net-gateway",
        "net-gateway-addresses",
        "net-gateway-accepted-chip",
        "net-gateway-programmed-chip",
        "net-listener-0",
        "net-listener-condition-0",
        "net-gateway-condition-1",
    ] {
        assert!(d.shown(selector), "{selector} is on screen");
    }
    d.f.dispatcher.clear();
    d.click("net-gateway-class-link");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: target(GATEWAY, "v1", "GatewayClass", None, "envoy")
        }],
        "Gateway to GatewayClass"
    );
    // And a GatewayClass has its own panel with its accepted condition.
    d.open(&target(GATEWAY, "v1", "GatewayClass", None, "envoy"));
    assert!(d.shown("net-gateway-class"));
    assert!(d.shown("net-gc-accepted-chip"));
    assert!(d.shown("net-gc-condition-0"));
}

#[gpui::test]
fn a_link_waits_for_discovery_instead_of_guessing_a_version(cx: &mut TestAppContext) {
    // The cluster serves no Gateway kind: nothing to resolve the route's parent against.
    let mut d = window(
        cx,
        vec![kind("", "v1", "Service", "services", true)],
        gateway_objects("v1"),
    );
    d.open(&target(GATEWAY, "v1", "HTTPRoute", Some("shop"), "web"));
    assert!(d.shown("net-route-parent-link-0"));
    d.f.dispatcher.clear();
    d.click("net-route-parent-link-0");
    assert!(
        d.f.dispatcher.sent().is_empty(),
        "an unplaceable link does nothing"
    );
}

/// How many times the cluster's discovery was run.
fn discover_calls(d: &Detail) -> usize {
    d.f.ports()
        .discovery
        .recorded_calls()
        .iter()
        .filter(|call| **call == DiscoveryCall::Discover)
        .count()
}

#[gpui::test]
fn a_failed_discovery_is_asked_again_so_the_links_come_back(cx: &mut TestAppContext) {
    let mut d = window(cx, served("v1"), gateway_objects("v1"));
    d.f.ports()
        .discovery
        .script()
        .discover
        .push_err(OxiError::network("connection refused"));
    d.f.ports().discovery.clear_calls();
    d.open(&target(GATEWAY, "v1", "HTTPRoute", Some("shop"), "web"));
    assert_eq!(discover_calls(&d), 1);
    d.f.dispatcher.clear();
    d.click("net-route-parent-link-0");
    assert!(
        d.f.dispatcher.sent().is_empty(),
        "no link while discovery is down"
    );

    d.f.vcx.executor().advance_clock(Duration::from_secs(6));
    d.settle();
    d.draw();
    assert_eq!(discover_calls(&d), 2, "asked again after the pause");
    d.click("net-route-parent-link-0");
    assert_eq!(d.f.dispatcher.sent().len(), 1, "the link works now");
}

#[gpui::test]
fn opening_more_details_does_not_run_discovery_again(cx: &mut TestAppContext) {
    let mut d = window(cx, served("v1"), gateway_objects("v1"));
    d.f.ports().discovery.clear_calls();
    d.open(&target(GATEWAY, "v1", "HTTPRoute", Some("shop"), "web"));
    d.open(&target(GATEWAY, "v1", "Gateway", Some("infra"), "edge"));
    d.open(&target(GATEWAY, "v1", "GatewayClass", None, "envoy"));
    d.open(&target(GATEWAY, "v1", "HTTPRoute", Some("shop"), "web"));
    assert_eq!(discover_calls(&d), 1, "one discovery serves every panel");
}

#[gpui::test]
fn only_the_ingress_class_row_action_is_offered_for_ingress_classes(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.discovery.set_kinds(served("v1"));
    ports.resources.insert(class("nginx", false));
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.settle();
    let classes = f.open(kind(
        NETWORKING,
        "v1",
        "IngressClass",
        "ingressclasses",
        false,
    ));
    f.keys(&classes, "j");
    let labels = |f: &mut Fixture, table: &gpui::Entity<crate::table::ResourceTable>| {
        f.vcx.update(|_, cx| {
            table
                .read(cx)
                .action_entries(cx)
                .into_iter()
                .map(|e| (e.label.clone(), e.is_enabled(), e.reason()))
                .collect::<Vec<_>>()
        })
    };
    let entries = labels(&mut f, &classes);
    assert!(
        entries
            .iter()
            .any(|(label, enabled, _)| label == "Set as default" && *enabled),
        "{entries:?}"
    );

    // Read-only: present, disabled, with the reason.
    f.sessions
        .set_read_only(&cluster(), true)
        .expect("open session");
    let entries = labels(&mut f, &classes);
    let set_default = entries
        .iter()
        .find(|(label, ..)| label == "Set as default")
        .expect("listed");
    assert!(!set_default.1);
    assert_eq!(set_default.2.as_deref(), Some("This cluster is read-only"));
    f.sessions
        .set_read_only(&cluster(), false)
        .expect("open session");

    // Running it sends the command for the selected row.
    f.dispatcher.clear();
    f.vcx.update(|window, cx| {
        classes.update(cx, |table, cx| {
            let targets = table.action_targets(cx);
            table.run_action(CommandId::INGRESS_CLASS_SET_DEFAULT, targets, window, cx);
        })
    });
    assert_eq!(
        f.dispatcher.sent(),
        [Command::IngressClassSetDefault {
            target: target(NETWORKING, "v1", "IngressClass", None, "nginx"),
            clear_previous: true,
        }]
    );

    // Pods do not get it.
    let pods = f.open_pods();
    let pod_labels = labels(&mut f, &pods);
    assert!(
        pod_labels
            .iter()
            .all(|(label, ..)| label != "Set as default"),
        "{pod_labels:?}"
    );
}

fn network_ids(f: &mut Fixture) -> Vec<String> {
    row_ids(f)
        .into_iter()
        .filter(|id| id.starts_with("network/"))
        .collect()
}

fn connected(cx: &mut TestAppContext, kinds: Vec<ResourceKind>) -> Fixture {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.access.set_rules(AccessRules::all_access());
    ports.discovery.set_kinds(kinds);
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.settle();
    f
}

#[gpui::test]
fn the_network_section_lists_ingress_kinds_always_and_the_gateway_api_only_where_served(
    cx: &mut TestAppContext,
) {
    let mut f = connected(cx, discovery_sets::core());
    let ids = network_ids(&mut f);
    for entry in [
        "network/services",
        "network/ingresses",
        "network/ingressclasses",
        "network/networkpolicies",
    ] {
        assert!(ids.contains(&entry.to_owned()), "{entry} in {ids:?}");
    }
    assert!(
        ids.iter()
            .all(|id| !id.contains("gateway") && !id.contains("route")),
        "no Gateway API entry, no placeholder: {ids:?}"
    );
}

#[gpui::test]
fn tls_routes_appear_at_any_served_version_and_the_gateway_kinds_leave_custom_resources(
    cx: &mut TestAppContext,
) {
    let mut kinds = discovery_sets::core_with_gateway_api();
    // TLSRoute is experimental: served at v1alpha2 here. ReferenceGrant is in the group but is not
    // one of the five the Network section lists.
    kinds.push(kind(GATEWAY, "v1alpha2", "TLSRoute", "tlsroutes", true));
    kinds.push(kind(
        GATEWAY,
        "v1beta1",
        "ReferenceGrant",
        "referencegrants",
        true,
    ));
    let mut f = connected(cx, kinds);
    let ids = network_ids(&mut f);
    for entry in [
        "network/gatewayclasses",
        "network/gateways",
        "network/httproutes",
        "network/grpcroutes",
        "network/tlsroutes",
    ] {
        assert!(ids.contains(&entry.to_owned()), "{entry} in {ids:?}");
    }
    // The group shows under Custom Resources for what the Network section does not list.
    let all = row_ids(&mut f);
    assert!(all.contains(&format!("crd:{GATEWAY}")), "{all:?}");

    // Without ReferenceGrant the group has nothing left for Custom Resources.
    let mut f = connected(cx, discovery_sets::core_with_gateway_api());
    let all = row_ids(&mut f);
    assert!(!all.contains(&format!("crd:{GATEWAY}")), "{all:?}");
}

#[gpui::test]
fn the_gateway_kinds_have_their_own_panels_and_columns_and_feeds_start_only_when_opened(
    cx: &mut TestAppContext,
) {
    let mut f = connected(cx, discovery_sets::core_with_gateway_api());
    let watches = |f: &mut Fixture| {
        f.ports()
            .resources
            .recorded_calls()
            .iter()
            .filter(
                |call| matches!(call, ResourceCall::Watch { kind, .. } if &*kind.group == GATEWAY),
            )
            .count()
    };
    assert_eq!(
        watches(&mut f),
        0,
        "no Gateway feed until the user opens the kind"
    );
    for kind in [
        "GatewayClass",
        "Gateway",
        "HTTPRoute",
        "GRPCRoute",
        "TLSRoute",
    ] {
        let has_panel = f.vcx.update(|_, cx| {
            !KindSpecs::detail_for(cx, &Gvk::new(GATEWAY, "v1", kind)).is_generic()
        });
        assert!(has_panel, "{kind} has its own panel");
    }

    f.ports().resources.insert(gateway_objects("v1").remove(1));
    let table = f.open(kind(GATEWAY, "v1", "Gateway", "gateways", true));
    assert!(watches(&mut f) >= 1, "opening the kind starts its feed");
    assert_eq!(f.names(&table), ["edge"]);
    let provider = oxikube_app::CoreColumns::new();
    let (columns, cells) = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let now = jiff::Timestamp::now();
            let cells: Vec<String> = d
                .rows()
                .first()
                .map(|row| {
                    ["addresses", "listeners", "programmed"]
                        .map(|id| {
                            let column = oxikube_app::ColumnId::new(id);
                            oxikube_app::ColumnProvider::cell(&provider, row, &column, now)
                                .display()
                                .to_owned()
                        })
                        .to_vec()
                })
                .unwrap_or_default();
            (d.layout().visible_ids(), cells)
        })
    });
    let ids: Vec<String> = columns.iter().map(|c| c.to_string()).collect();
    assert_eq!(
        ids,
        [
            "name",
            "namespace",
            "class",
            "addresses",
            "listeners",
            "programmed",
            "age"
        ]
    );
    assert_eq!(cells, ["203.0.113.7", "HTTP :80", "False"]);
}
