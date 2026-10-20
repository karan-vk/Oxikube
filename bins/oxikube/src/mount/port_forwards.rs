//! The app's one [`PortForwardManager`] (E15-S03): forwards to pods and Services across clusters
//! that follow pod churn, over the sessions and the shared resource stores; its supervisors run on
//! the Tokio bridge, its backoff on the app clock; drops, recoveries and failures go to the
//! notification centre.
//!
//! Installing it also turns on the Service panel's "Open in browser" buttons (the manager is the
//! opener behind `service::OpenInBrowser`, registered on the bus by `mount::bus`): a user reaches
//! a forward from a Service's detail (a port's "Open in browser"), from `o` on a Service row, from
//! the palette ("Open Service in Browser", "Start Port-Forward") and as MCP tools.

use std::sync::Arc;

use gpui::App;
use oxikube_app::{PortForwardManager, ResourceStores, SessionBackend};
use oxikube_resources_ui::kinds::service::ServiceBrowser;

use crate::app_state::{AppState, ClusterServices};

/// The manager already set on `state`, else a new one that is set; enables the Service panel's
/// browser buttons.
pub fn manager(
    state: &AppState,
    services: &ClusterServices,
    stores: Arc<ResourceStores>,
    cx: &mut App,
) -> PortForwardManager {
    ServiceBrowser::enable(cx);
    if let Some(manager) = state.port_forwards() {
        return manager.clone();
    }
    let clock = state.ports().clusters.clock.clone();
    let spawner = oxikube_resources_ui::table::store_runtime(clock.clone(), cx).spawner;
    let manager = PortForwardManager::new(
        Arc::new(SessionBackend::new(services.sessions.clone(), stores)),
        spawner,
        clock,
        Some(services.notifications.sources()),
    );
    if !state.set_port_forwards(manager.clone()) {
        return state.port_forwards().cloned().unwrap_or(manager);
    }
    manager
}
