//! Network: Services, Endpoints and EndpointSlices. Ingresses, IngressClasses, NetworkPolicies and
//! the Gateway API have their own module (`kinds::network`, E12-S14).

use gpui::App;
use oxikube_app::Section;
use oxikube_domain::command::CommandId;

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
