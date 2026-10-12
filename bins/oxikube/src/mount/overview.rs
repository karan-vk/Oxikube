//! The cluster overview in the main window (E13-S09).
//!
//! * [`deps`] builds what every overview reads: the sessions and stores, the bus, the live metrics
//!   service (usage, top consumers), the history metrics (the provider service, the PromQL port and
//!   the query catalogue: the graphs) and the warnings hook the Workloads overview reads too (none until the Events feed, E14).
//! * [`install`] runs for every cluster tab: the overview is the tab's first screen once the
//!   cluster is connected (when nothing else is open), the sidebar's "Cluster" header (the
//!   `overview` page) opens it, and its "Cluster overview" entry (Metrics section) runs
//!   `cluster::ShowOverview`.
//! * [`start_views`] applies each `cluster::ShowOverview` (the palette's "Cluster Overview", the
//!   sidebar entry, agents) on the UI thread.
//! * [`follow_tabs`] tells every overview whether its cluster's tab is the one the window shows:
//!   a hidden tab's overview releases its feeds.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Subscription, Task, WeakEntity, Window};
use oxikube_app::{ClusterSession, RecentWarnings};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_overview_ui::{ClusterOverview, HistoryDeps, OverviewDeps, OverviewViews};
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, ClusterTabs, ClusterTabsEvent, CommandDispatcher};

use crate::app_state::ClusterServices;

/// The page id the sidebar's "Cluster" section navigates to.
pub const OVERVIEW_PAGE: &str = "overview";

/// What every overview of the window reads. See the [module docs](self).
pub fn deps(
    services: &ClusterServices,
    stores: Arc<oxikube_app::ResourceStores>,
    dispatcher: Rc<dyn CommandDispatcher>,
    warnings: Arc<dyn RecentWarnings>,
) -> OverviewDeps {
    OverviewDeps {
        sessions: services.sessions.clone(),
        stores,
        dispatcher,
        metrics: Some(services.live_metrics.clone()),
        history: Some(HistoryDeps {
            source: Arc::new(services.metrics.provider.clone()),
            promql: services.metrics.promql.clone(),
            catalogue: services.metrics.catalogue.clone(),
        }),
        warnings,
    }
}

/// Sets up one cluster tab's overview. See the [module docs](self).
pub fn install(
    tab: &Entity<ClusterTab>,
    session: &ClusterSession,
    overview: &OverviewDeps,
    window: &mut Window,
    cx: &mut App,
) {
    let cluster = session.id().clone();
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
                // The cluster may connect while another tab is shown: its overview then waits,
                // holding no feeds, until `follow_tabs` reports the tab shown.
                let shown = tab.read(cx).is_active();
                ClusterOverview::open(
                    &workspace,
                    cluster.clone(),
                    overview.clone(),
                    shown,
                    window,
                    cx,
                );
            }
        }
    };
    open_first(window, cx);
    window
        .observe(tab, cx, move |_, window, cx| open_first(window, cx))
        .detach();

    let Some(panel) = workspace.read(cx).panel::<SidebarPanel>() else {
        tracing::warn!(%cluster, "the cluster tab has no sidebar: its overview entries do not navigate");
        return;
    };
    let overview = overview.clone();
    let tab = tab.downgrade();
    window
        .subscribe(&panel, cx, move |_, event: &SidebarEvent, window, cx| {
            let SidebarEvent::Navigate(target) = event;
            match target {
                SidebarTarget::Page(page) if page.as_ref() == OVERVIEW_PAGE => {
                    let shown = tab.upgrade().is_some_and(|tab| tab.read(cx).is_active());
                    ClusterOverview::open(
                        &workspace,
                        cluster.clone(),
                        overview.clone(),
                        shown,
                        window,
                        cx,
                    );
                }
                SidebarTarget::Command(id) if *id == CommandId::CLUSTER_SHOW_OVERVIEW => {
                    let cluster = cluster.clone();
                    overview
                        .dispatcher
                        .dispatch(Command::ClusterShowOverview { cluster }, cx);
                }
                _ => {}
            }
        })
        .detach();
}

/// Applies the bus's `cluster::ShowOverview` requests. Keep the task for the life of the window.
pub fn start_views(
    deps: OverviewDeps,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<ClusterId>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    OverviewViews::start(deps, tabs, requests, window, cx)
}

/// Tells every overview whether its cluster's tab is shown, now and on every tab switch. Keep
/// the subscription for the life of the window.
pub fn follow_tabs(tabs: &Entity<ClusterTabs>, cx: &mut App) -> Subscription {
    show(tabs, tabs.read(cx).active().cloned(), cx);
    cx.subscribe(tabs, |tabs, event: &ClusterTabsEvent, cx| {
        if let ClusterTabsEvent::ActiveChanged(active) = event {
            show(&tabs, active.clone(), cx);
        }
    })
}

fn show(tabs: &Entity<ClusterTabs>, active: Option<ClusterId>, cx: &mut App) {
    let workspaces: Vec<_> = {
        let tabs = tabs.read(cx);
        tabs.clusters(cx)
            .into_iter()
            .filter_map(|cluster| {
                let tab = tabs.tab(&cluster)?;
                Some((cluster, tab.read(cx).workspace().clone()))
            })
            .collect()
    };
    for (cluster, workspace) in workspaces {
        let shown = active.as_ref() == Some(&cluster);
        for overview in workspace.read(cx).items_of_type::<ClusterOverview>() {
            overview.update(cx, |o, cx| o.set_tab_shown(shown, cx));
        }
    }
}
