//! The Helm charts catalog of the main window (E16-S08): `helm::OpenCharts` opens it in the active
//! cluster's tab, and the sidebar's Helm > Charts entry asks for it.
//!
//! * the [`ChartsService`](oxikube_app::ChartsService) is built with the repositories service
//!   ([`super::helm_repos::services`]), over the same `helm` adapter.
//! * [`start_views`] applies each `helm::OpenCharts` request on the UI thread
//!   (`oxikube_helm_ui::charts::HelmChartsViews`).
//! * [`install_tab`] runs for every cluster tab: a click on the sidebar's Charts entry is the same
//!   command.

use std::rc::Rc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Task, WeakEntity, Window};
use oxikube_app::ChartsService;
use oxikube_domain::command::{Command, CommandId};
use oxikube_helm_ui::charts::{HelmChartsViews, HelmChartsViewsDeps, ServiceBackend};
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, ClusterTabs, CommandDispatcher};

use super::tabs::TabDeps;

/// Starts the window's charts page over `requests` (the bus's `helm::OpenCharts` handler). Keep
/// the task for the life of the window.
pub fn start_views(
    service: ChartsService,
    dispatcher: Rc<dyn CommandDispatcher>,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<()>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let deps = HelmChartsViewsDeps {
        backend: Rc::new(ServiceBackend::new(service)),
        dispatcher,
        tabs,
    };
    HelmChartsViews::start(deps, requests, window, cx)
}

/// Routes a click on the sidebar's Charts entry to `helm::OpenCharts`.
pub fn install_tab(tab: &Entity<ClusterTab>, deps: &TabDeps, window: &mut Window, cx: &mut App) {
    let workspace = tab.read(cx).workspace().clone();
    let Some(panel) = workspace.read(cx).panel::<SidebarPanel>() else {
        tracing::warn!("the cluster tab has no sidebar: its Helm charts entry does not navigate");
        return;
    };
    let dispatcher = deps.dispatcher.clone();
    window
        .subscribe(&panel, cx, move |_, event: &SidebarEvent, _, cx| {
            if matches!(
                event,
                SidebarEvent::Navigate(SidebarTarget::Command(id)) if *id == CommandId::HELM_OPEN_CHARTS
            ) {
                dispatcher.dispatch(Command::HelmOpenCharts, cx);
            }
        })
        .detach();
}
