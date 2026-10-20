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

use gpui::{App, Global, Task};
use oxikube_app::portforward::PortForwardFavourites;
use oxikube_app::{PortForwardManager, ResourceStores, SessionBackend};
use oxikube_resources_ui::kinds::service::ServiceBrowser;

use crate::app_state::{AppState, ClusterServices};

/// Keeps the favourites' follower alive for the app: it restores a cluster's favourites when its
/// session becomes `Ready` (E15-S06).
struct FavouritesFollow(#[allow(dead_code)] Task<()>);

impl Global for FavouritesFollow {}

/// The manager already set on `state`, else a new one that is set; enables the Service panel's
/// browser buttons. Also the saved port-forwards, which start again when their cluster connects.
pub fn manager(
    state: &AppState,
    services: &ClusterServices,
    stores: Arc<ResourceStores>,
    cx: &mut App,
) -> (PortForwardManager, PortForwardFavourites) {
    ServiceBrowser::enable(cx);
    if let (Some(manager), Some(favourites)) =
        (state.port_forwards(), state.port_forward_favourites())
    {
        return (manager.clone(), favourites.clone());
    }
    let clock = state.ports().clusters.clock.clone();
    let spawner = oxikube_resources_ui::table::store_runtime(clock.clone(), cx).spawner;
    let manager = PortForwardManager::new(
        Arc::new(SessionBackend::new(services.sessions.clone(), stores)),
        spawner.clone(),
        clock.clone(),
        Some(services.notifications.sources()),
    );
    let favourites = PortForwardFavourites::new(
        state.ports().state.clone(),
        manager.clone(),
        spawner,
        clock,
        Some(services.notifications.sources()),
    );
    // Both are set together on the first mount; a racing second window keeps the first's.
    if !state.set_port_forwards(manager.clone()) {
        return (
            state.port_forwards().cloned().unwrap_or(manager),
            state
                .port_forward_favourites()
                .cloned()
                .unwrap_or(favourites),
        );
    }
    state.set_port_forward_favourites(favourites.clone());
    let follow = favourites.follow(&services.sessions);
    let task = cx.spawn(async move |_| follow.await);
    cx.set_global(FavouritesFollow(task));
    (manager, favourites)
}
