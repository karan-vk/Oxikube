//! The resource side of the main window (E07-S11): the stores every cluster tab reads, what a
//! cluster tab gets for them (the Workloads overview, navigation from the sidebar), and the
//! requests `resource::OpenList` sends.
//!
//! * [`stores`] builds the app's one `ResourceStores` on first use: feeds run on the Tokio bridge
//!   (on GPUI's background executor in the deterministic test runtime), grace timers and
//!   back-off on the app clock.
//! * [`install`] runs for every cluster tab: it opens the [`WorkloadsOverview`] as the tab's first
//!   screen once the cluster is connected (when nothing else is open), and routes the sidebar's
//!   "Cluster" section to the overview. A kind entry is the resource views' (E07-S03,
//!   `oxikube_resources_ui::sidebar_navigation`): they find the kind through discovery, custom
//!   resources included, and send `resource::OpenList` on the bus.
//! * [`open_kinds`] applies the `resource::OpenList` requests on the UI thread: the views
//!   registered with `oxikube_resources_ui::navigate::KindViews` (the generic resource table of
//!   E07-S03) open the list, or a notice says that none can.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Task, Window};
use oxikube_app::{ClusterSession, ResourceStores};
use oxikube_ports::ClockPort;
use oxikube_resources_ui::navigate::{OpenKind, open_kind};
use oxikube_resources_ui::overview_lite::{OverviewDeps, WorkloadsOverview};
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, ClusterTabs, Toast, Workspace};

use super::tabs::TabDeps;
use crate::app_state::AppState;

/// The page id the sidebar's "Cluster" section navigates to.
const OVERVIEW_PAGE: &str = "overview";

/// The app's `ResourceStores`: the ones already set on `state`, else new ones that are set.
pub fn stores(state: &AppState, clock: Arc<dyn ClockPort>, cx: &App) -> Arc<ResourceStores> {
    if let Some(stores) = state.resource_stores() {
        return stores.clone();
    }
    let runtime = oxikube_resources_ui::table::store_runtime(clock, cx);
    let stores = Arc::new(ResourceStores::new(runtime));
    if !state.set_resource_stores(stores.clone()) {
        // Another window set them first: use those so every window shares the feeds.
        return state.resource_stores().cloned().unwrap_or(stores);
    }
    stores
}

/// Sets up one cluster tab's resource views. See the [module docs](self).
pub fn install(
    tab: &Entity<ClusterTab>,
    session: &ClusterSession,
    deps: &TabDeps,
    window: &mut Window,
    cx: &mut App,
) {
    let cluster = session.id().clone();
    let overview = OverviewDeps {
        sessions: deps.sessions.clone(),
        stores: deps.stores.clone(),
        dispatcher: deps.dispatcher.clone(),
    };
    let workspace = tab.read(cx).workspace().clone();

    // The first screen: the overview, once the cluster is connected and nothing else is open.
    let opened = Rc::new(Cell::new(false));
    let open_first = {
        let (tab, workspace) = (tab.clone(), workspace.clone());
        let (cluster, overview) = (cluster.clone(), overview.clone());
        move |window: &mut Window, cx: &mut App| {
            if opened.get() || !tab.read(cx).info().state.phase().is_connected() {
                return;
            }
            opened.set(true);
            if workspace.read(cx).items().next().is_none() {
                WorkloadsOverview::open(&workspace, cluster.clone(), overview.clone(), window, cx);
            }
        }
    };
    open_first(window, cx);
    let observe = window.observe(tab, cx, move |_, window, cx| open_first(window, cx));
    observe.detach();

    // The sidebar's clicks.
    let Some(panel) = workspace.read(cx).panel::<SidebarPanel>() else {
        tracing::warn!(%cluster, "the cluster tab has no sidebar: its entries do not navigate");
        return;
    };
    let navigate = window.subscribe(&panel, cx, move |_, event: &SidebarEvent, window, cx| {
        let SidebarEvent::Navigate(target) = event;
        match target {
            SidebarTarget::Page(page) if page.as_ref() == OVERVIEW_PAGE => {
                WorkloadsOverview::open(&workspace, cluster.clone(), overview.clone(), window, cx);
            }
            // Kind entries: `oxikube_resources_ui::sidebar_navigation` (see the module docs).
            SidebarTarget::Page(_) | SidebarTarget::Kind { .. } | SidebarTarget::Command(_) => {}
        }
    });
    navigate.detach();
}

/// Applies each `resource::OpenList` request on the UI thread: finds the cluster's tab and asks
/// the registered kind views to open the list in it; says so when none can.
pub fn open_kinds(
    mut requests: UnboundedReceiver<OpenKind>,
    tabs: Entity<ClusterTabs>,
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let workspace = workspace.downgrade();
    window.spawn(cx, async move |cx| {
        while let Some(request) = requests.next().await {
            let Some(window_workspace) = workspace.upgrade() else {
                break;
            };
            let handled = cx.update(|window, cx| {
                let tab_workspace = tabs
                    .read(cx)
                    .tab(&request.cluster)
                    .map(|tab| tab.read(cx).workspace().clone());
                let opened = tab_workspace.is_some_and(|ws| open_kind(&request, &ws, window, cx));
                if !opened {
                    let kind = request.gvk.kind.to_string();
                    window_workspace.update(cx, |ws, cx| {
                        ws.show_toast(
                            Toast::info(format!("There is no list view for {kind} yet.")),
                            cx,
                        );
                    });
                }
            });
            if handled.is_err() {
                break;
            }
        }
    })
}
