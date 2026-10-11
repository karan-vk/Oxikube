//! Network: Services, Endpoints, EndpointSlices, Ingresses, IngressClasses, NetworkPolicies and
//! the Gateway API (optional: shown where discovery serves it).

use gpui::App;
use oxikube_app::Section;
use oxikube_domain::command::CommandId;

use super::{Builtin, cluster, generic, namespaced, register};

/// The Gateway API's group.
const GATEWAY: &str = "gateway.networking.k8s.io";

const NETWORK: &[Builtin] = &[
    namespaced("", "v1", "Service", "services", "Services"),
    namespaced("", "v1", "Endpoints", "endpoints", "Endpoints"),
    namespaced(
        "discovery.k8s.io",
        "v1",
        "EndpointSlice",
        "endpointslices",
        "Endpoint Slices",
    ),
    namespaced(
        "networking.k8s.io",
        "v1",
        "Ingress",
        "ingresses",
        "Ingresses",
    ),
    cluster(
        "networking.k8s.io",
        "v1",
        "IngressClass",
        "ingressclasses",
        "Ingress Classes",
    ),
    namespaced(
        "networking.k8s.io",
        "v1",
        "NetworkPolicy",
        "networkpolicies",
        "Network Policies",
    ),
    cluster(
        GATEWAY,
        "v1",
        "GatewayClass",
        "gatewayclasses",
        "Gateway Classes",
    ),
    namespaced(GATEWAY, "v1", "Gateway", "gateways", "Gateways"),
    namespaced(GATEWAY, "v1", "HTTPRoute", "httproutes", "HTTP Routes"),
    namespaced(GATEWAY, "v1", "GRPCRoute", "grpcroutes", "GRPC Routes"),
    namespaced(GATEWAY, "v1alpha2", "TLSRoute", "tlsroutes", "TLS Routes"),
];

/// A Service's own actions (E12-S13): its pods by selector, and its ports in the browser.
const SERVICE_ACTIONS: &[CommandId] = &[
    CommandId::SERVICE_SHOW_PODS,
    CommandId::SERVICE_OPEN_IN_BROWSER,
];

/// Registers the Network section's kinds.
pub fn init(cx: &mut App) {
    register(cx, Section::NETWORK, NETWORK, |kind| {
        if kind.kind == "Service" && kind.group.is_empty() {
            SERVICE_ACTIONS
        } else {
            generic(kind)
        }
    });
    crate::kinds::service::init(cx);
    crate::kinds::endpoints::init(cx);
}
