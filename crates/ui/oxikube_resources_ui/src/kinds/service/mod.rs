//! Services (E12-S13): the Service's detail panel, the open-in-browser hook and the Service row
//! actions. The columns (Type, Cluster IP, External IP, Ports, Selector, Status) are the
//! catalogue's in `oxikube_app::columns`, read through the shared provider; the rules behind
//! them are `oxikube_domain::view::ServiceSummary`.
//!
//! | File | Holds |
//! |---|---|
//! | `model` | `ServiceView`: the panel's content built from the Service, its EndpointSlices and its pods, off the render path and capped |
//! | `feeds` | the two label-selected subscriptions of the panel ([`oxikube_app::ServiceBackends`]) |
//! | `panel` | [`ServicePanel`]: the entity above the Overview, following the detail it sits in |
//! | `render` | drawing the panel: facts, selector chips, ports with "Open in browser" |
//! | `lists` | the panel's EndpointSlices and backing pods |
//! | `browser` | [`ServiceBrowser`] and [`register_open_in_browser`]: the hook E15's port-forward manager installs |
//! | `actions` | [`service_row_actions`]: "Show Pods" and "Open in Browser" in a Service row's menu and the palette |
//!
//! # How a user gets here
//!
//! Network → Services in the cluster sidebar lists the Services; Enter on a row (or a double
//! click) opens the detail drawer, whose panel lists the ports, the EndpointSlices and the
//! backing pods. `u` on a Service row, "Show pods" in the panel, the row menu and the palette's
//! "Show Service Pods" open the Pods table filtered by the selector. "Open in browser" appears
//! once the port-forward manager (E15) installs its opener.
//!
//! # Performance
//!
//! Not a hot path. The panel starts two server-filtered feeds when a Service's detail opens and
//! drops them when it closes; the model is rebuilt per delta batch (coalesced), never per frame,
//! and caps its lists.

mod actions;
mod browser;
mod feeds;
mod lists;
mod model;
mod panel;
mod render;

#[cfg(test)]
mod tests;

pub use actions::{OPEN_IN_BROWSER_ORDER, SHOW_PODS_ORDER, service_row_actions};
pub use browser::{ServiceBrowser, register_open_in_browser};
pub use panel::ServicePanel;

use gpui::App;

use super::DetailRenderer;

/// The detail renderer of Services: the [`ServicePanel`] above the generic Overview.
pub fn detail() -> DetailRenderer {
    DetailRenderer::panel(panel::build)
}

/// Gives the Service kind its panel (the kind is registered with the Network section).
pub(super) fn init(cx: &mut App) {
    super::KindSpecs::set_detail(
        cx,
        &oxikube_domain::ids::Gvk::new("", "v1", "Service"),
        detail(),
    );
}
