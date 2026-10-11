//! The metrics page of the main window (E13-S05): `cluster::ShowMetricsSettings` opens it in the
//! cluster's tab, and the sidebar's Metrics entry asks for it.
//!
//! * [`start_views`] applies each `cluster::ShowMetricsSettings` request on the UI thread
//!   (`oxikube_settings_ui::metrics::MetricsViews`), over the app's metrics provider service.
//! * [`install_tab`] runs for every cluster tab: a click on the sidebar's "Provider and settings"
//!   entry is the same command, with the tab's cluster.
//!
//! Starting the service itself (it follows the sessions and the settings) is
//! [`crate::metrics_provider::start`], called once by the mount.

use std::rc::Rc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Task, WeakEntity, Window};
use oxikube_app::{ClusterSession, ClusterSessionManager, MetricsProviderService};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_settings_ui::metrics::{MetricsViews, MetricsViewsDeps, ServiceBackend};
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, ClusterTabs, CommandDispatcher};

use super::tabs::TabDeps;

/// Starts the window's metrics pages over `requests` (the bus's `cluster::ShowMetricsSettings`
/// handler). Keep the task for the life of the window.
pub fn start_views(
    provider: MetricsProviderService,
    sessions: ClusterSessionManager,
    dispatcher: Rc<dyn CommandDispatcher>,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<ClusterId>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let deps = MetricsViewsDeps {
        backend: Rc::new(ServiceBackend::new(provider, sessions)),
        dispatcher,
        tabs,
    };
    MetricsViews::start(deps, requests, window, cx)
}

/// Routes a click on the sidebar's metrics entry to `cluster::ShowMetricsSettings` for this tab's
/// cluster.
pub fn install_tab(
    tab: &Entity<ClusterTab>,
    session: &ClusterSession,
    deps: &TabDeps,
    window: &mut Window,
    cx: &mut App,
) {
    let cluster = session.id().clone();
    let workspace = tab.read(cx).workspace().clone();
    let Some(panel) = workspace.read(cx).panel::<SidebarPanel>() else {
        tracing::warn!(%cluster, "the cluster tab has no sidebar: its metrics entry does not navigate");
        return;
    };
    let dispatcher = deps.dispatcher.clone();
    window
        .subscribe(&panel, cx, move |_, event: &SidebarEvent, _, cx| {
            let SidebarEvent::Navigate(SidebarTarget::Command(id)) = event else {
                return;
            };
            if *id == CommandId::CLUSTER_SHOW_METRICS_SETTINGS {
                let cluster = cluster.clone();
                dispatcher.dispatch(Command::ClusterShowMetricsSettings { cluster }, cx);
            }
        })
        .detach();
}
