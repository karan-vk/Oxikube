//! Network: Services, Endpoints and EndpointSlices. Ingresses, IngressClasses, NetworkPolicies and
//! the Gateway API have their own module (`kinds::network`, E12-S14).

use gpui::App;
use oxikube_app::Section;

use super::{Builtin, generic, namespaced, register};

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
];

/// Registers the Network section's kinds.
pub fn init(cx: &mut App) {
    register(cx, Section::NETWORK, NETWORK, generic);
}
