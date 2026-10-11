//! The Network section's kinds that have their own columns and panels (E12-S14): Ingress,
//! IngressClass, NetworkPolicy and the Gateway API (GatewayClass, Gateway, HTTPRoute, GRPCRoute,
//! TLSRoute). Services, Endpoints and EndpointSlices have their own modules (`kinds::service`, `kinds::endpoints`, E12-S13).
//!
//! | File | Holds |
//! |---|---|
//! | `panel` | `NetworkPanel`: the one entity every kind's panel is; it keeps the parsed view-model of the detail's object until the object changes |
//! | `ingress`, `ingress_class`, `policy`, `gateway`, `route` | what each kind's panel draws (`impl NetworkPanel` blocks) |
//! | `widgets` | headings, fields, chips, table rows and condition lines |
//! | `nav` | the version and scope a referenced kind is served at (from discovery), for the links |
//! | `actions` | the row action "Set as default" of IngressClass rows |
//!
//! The columns are the core catalogue's (`oxikube_app::columns`, computed from the view-models in
//! `oxikube_domain::view::network`), so they are the same for a table, the palette and an agent.
//!
//! # Where the Gateway API appears
//!
//! The five Gateway API kinds are registered in the Network section like the others; the cluster
//! sidebar shows them only where discovery serves the group (`gateway.networking.k8s.io`), and
//! shows nothing, no error and no empty section, where it does not. The Custom Resources section
//! leaves these five out (`oxikube_app::sidebar::custom_resource_groups`). Their feeds start
//! when the user opens the kind. The served version is never assumed: the table and the links
//! resolve it from discovery.

mod actions;
mod gateway;
mod ingress;
mod ingress_class;
mod nav;
mod panel;
mod policy;
mod route;
mod widgets;

use gpui::App;
use oxikube_app::Section;
use oxikube_domain::command::CommandId;

pub use actions::network_row_actions;

use super::builtin::{Builtin, cluster, namespaced, spec};
use super::detail::DetailRenderer;
use super::specs::KindSpecs;
use panel::NetworkPanel;

/// The Gateway API's group.
const GATEWAY: &str = "gateway.networking.k8s.io";

/// The kinds of this module and their sidebar rank (Services 10, Endpoints 20 and EndpointSlices
/// 30 are `builtin::network`'s).
const KINDS: &[(Builtin, u32)] = &[
    (
        namespaced(
            "networking.k8s.io",
            "v1",
            "Ingress",
            "ingresses",
            "Ingresses",
        ),
        40,
    ),
    (
        cluster(
            "networking.k8s.io",
            "v1",
            "IngressClass",
            "ingressclasses",
            "Ingress Classes",
        ),
        50,
    ),
    (
        namespaced(
            "networking.k8s.io",
            "v1",
            "NetworkPolicy",
            "networkpolicies",
            "Network Policies",
        ),
        60,
    ),
    (
        cluster(
            GATEWAY,
            "v1",
            "GatewayClass",
            "gatewayclasses",
            "Gateway Classes",
        ),
        70,
    ),
    (
        namespaced(GATEWAY, "v1", "Gateway", "gateways", "Gateways"),
        80,
    ),
    (
        namespaced(GATEWAY, "v1", "HTTPRoute", "httproutes", "HTTP Routes"),
        90,
    ),
    (
        namespaced(GATEWAY, "v1", "GRPCRoute", "grpcroutes", "GRPC Routes"),
        100,
    ),
    // Still experimental in many releases: the version here is only the one this was written
    // against; discovery decides which one a cluster serves.
    (
        namespaced(GATEWAY, "v1alpha2", "TLSRoute", "tlsroutes", "TLS Routes"),
        110,
    ),
];

/// Registers the kinds of this module in the Network section, each with its detail panel, and
/// `ingressclass::SetDefault` as the IngressClass action.
pub fn init(cx: &mut App) {
    let specs = KINDS.iter().map(|(kind, rank)| {
        let actions: &[CommandId] = if kind.kind == "IngressClass" {
            &[CommandId::INGRESS_CLASS_SET_DEFAULT]
        } else {
            &[]
        };
        spec(kind, Section::NETWORK, *rank, actions)
            .with_detail(DetailRenderer::panel(NetworkPanel::build))
    });
    if let Err(error) = KindSpecs::register_all(cx, specs) {
        tracing::error!(%error, "the Network kinds were not all registered");
    }
}
