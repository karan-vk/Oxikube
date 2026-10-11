//! Endpoints and EndpointSlices (E12-S13): the detail panel that lists their endpoints with
//! readiness, the node and the pod behind each, and a way back to the Service.
//!
//! Their columns (`Endpoints` with a "+ N more" count; `Address Type`, `Ports`, `Endpoints` for a
//! slice) are the catalogue's; the rules behind the panel are `oxikube_domain::view::
//! {EndpointSliceSummary, EndpointsSummary}`. The core `Endpoints` API is deprecated, but still
//! served, so the kind stays; the Service's own panel prefers the slices.
//!
//! | File | Holds |
//! |---|---|
//! | `model` | [`EndpointsView`](model::EndpointsView): the rows and counts, capped |
//! | `panel` | [`EndpointsPanel`]: the entity above the Overview |
//!
//! No feed starts: the panel reads the object the detail already follows.

mod model;
mod panel;

#[cfg(test)]
mod tests;

pub use panel::EndpointsPanel;

use gpui::App;
use oxikube_domain::ids::Gvk;

use super::{DetailRenderer, KindSpecs};

/// The detail renderer of Endpoints and EndpointSlices.
pub fn detail() -> DetailRenderer {
    DetailRenderer::panel(panel::build)
}

/// Gives both kinds their panel (they are registered with the Network section).
pub(super) fn init(cx: &mut App) {
    for gvk in [
        Gvk::new("", "v1", "Endpoints"),
        Gvk::new("discovery.k8s.io", "v1", "EndpointSlice"),
    ] {
        KindSpecs::set_detail(cx, &gvk, detail());
    }
}
