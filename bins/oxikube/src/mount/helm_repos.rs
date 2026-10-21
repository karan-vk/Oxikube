//! The Helm repositories page of the main window (E16-S07): `helm::ShowRepos` opens it in the
//! active cluster's tab, and the sidebar's Helm > Repositories entry asks for it.
//!
//! * [`service`] builds the app's one [`HelmReposService`] over the `helm` CLI adapter. Nothing
//!   runs at start-up: the adapter looks for the binary the first time the page reads the list
//!   (on the Tokio bridge), so a machine without Helm costs nothing and the page explains it.
//! * [`start_views`] applies each `helm::ShowRepos` request on the UI thread
//!   (`oxikube_settings_ui::helm_repos::HelmReposViews`).
//! * [`install_tab`] runs for every cluster tab: a click on the sidebar's Repositories entry is
//!   the same command.

use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Task, WeakEntity, Window};
use oxikube_app::{ChartsService, HelmReposService};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_helm::{HelmCli, HelmCliConfig};
use oxikube_helm_ui::repos::ServiceBackend;
use oxikube_ports::ClockPort;
use oxikube_settings_ui::helm_repos::{HelmReposViews, HelmReposViewsDeps};
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, ClusterTabs, CommandDispatcher};

use super::tabs::TabDeps;

/// The Helm services of the window: the repositories (E16-S07) and the charts catalog (E16-S08)
/// share one adapter, so the binary is looked for once.
#[derive(Clone)]
pub struct HelmServices {
    /// The chart repositories in Helm's own configuration.
    pub repos: HelmReposService,
    /// The charts catalog, dropping its caches when `repos` changes something.
    pub charts: ChartsService,
}

/// Services the tests install in place of the ones over the real `helm` binary.
#[cfg(test)]
pub struct ServiceOverride(pub HelmServices);

#[cfg(test)]
impl gpui::Global for ServiceOverride {}

/// The Helm services over the `helm` binary (found on first use). Repository commands and the
/// catalog do not touch a cluster, so no kubeconfig context is resolved.
pub fn services(clock: Arc<dyn ClockPort>, cx: &App) -> HelmServices {
    #[cfg(test)]
    if let Some(service) = cx.try_global::<ServiceOverride>() {
        return service.0.clone();
    }
    let _ = cx;
    let cli = Arc::new(HelmCli::new(
        HelmCliConfig::default(),
        Arc::new(|_: &ClusterId| None),
    ));
    let repos = HelmReposService::new(cli.clone(), cli.clone(), clock);
    let charts = ChartsService::new(cli, repos.catalog_changes());
    HelmServices { repos, charts }
}

/// Starts the window's repositories page over `requests` (the bus's `helm::ShowRepos` handler).
/// Keep the task for the life of the window.
pub fn start_views(
    service: HelmReposService,
    dispatcher: Rc<dyn CommandDispatcher>,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<()>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let deps = HelmReposViewsDeps {
        backend: Rc::new(ServiceBackend::new(service)),
        dispatcher,
        tabs,
    };
    HelmReposViews::start(deps, requests, window, cx)
}

/// Routes a click on the sidebar's Repositories entry to `helm::ShowRepos`.
pub fn install_tab(tab: &Entity<ClusterTab>, deps: &TabDeps, window: &mut Window, cx: &mut App) {
    let workspace = tab.read(cx).workspace().clone();
    let Some(panel) = workspace.read(cx).panel::<SidebarPanel>() else {
        tracing::warn!(
            "the cluster tab has no sidebar: its Helm repositories entry does not navigate"
        );
        return;
    };
    let dispatcher = deps.dispatcher.clone();
    window
        .subscribe(&panel, cx, move |_, event: &SidebarEvent, _, cx| {
            if matches!(
                event,
                SidebarEvent::Navigate(SidebarTarget::Command(id)) if *id == CommandId::HELM_SHOW_REPOS
            ) {
                dispatcher.dispatch(Command::HelmShowRepos, cx);
            }
        })
        .detach();
}
