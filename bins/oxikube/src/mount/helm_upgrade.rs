//! The Helm upgrade tab of the main window (E16-S10): every `helm::UpgradeRelease` (the sidebar's
//! Helm > Upgrade entry, the palette's "Helm: Upgrade Release...", and the release detail's
//! Upgrade button, which names the release) opens it in the active cluster's tab; its preview and upgrade are `helm::Upgrade` dispatches
//! through the guard.
//!
//! * [`start_views`] opens a tab per request (`oxikube_helm_ui::upgrade::HelmUpgradeViews`).
//! * [`install_tab`] runs for every cluster tab: a click on the sidebar's Upgrade entry is the same
//!   command.

use std::rc::Rc;

use futures::StreamExt as _;
use gpui::{App, Entity, Task, WeakEntity, Window};
use oxikube_app::{ChartsService, UpgradeFlow, UpgradeRequests};
use oxikube_domain::command::{Command, CommandId};
use oxikube_helm_ui::upgrade::{FlowBackend, HelmUpgradeViews, HelmUpgradeViewsDeps};
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, ClusterTabs};

use super::tabs::TabDeps;

/// Starts the window's upgrade tabs: one per release the commands ask to upgrade. Keep the task
/// for the life of the window.
pub fn start_views(
    flow: UpgradeFlow,
    charts: ChartsService,
    requests: &UpgradeRequests,
    tabs: WeakEntity<ClusterTabs>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    // Every request after this point (the watch's current value is an older request).
    let mut watch = requests.subscribe();
    watch.mark_unchanged();
    let requests = futures::stream::unfold(watch, |mut watch| async move {
        watch.changed().await.ok()?;
        let target = watch.borrow_and_update().clone();
        Some((target, watch))
    })
    .filter_map(|target| async move { target })
    .boxed();
    let deps = HelmUpgradeViewsDeps {
        backend: Rc::new(FlowBackend::new(flow, charts)),
        tabs,
    };
    HelmUpgradeViews::start(deps, requests, window, cx)
}

/// Routes a click on the sidebar's Upgrade entry to `helm::UpgradeRelease` (the tab then lists the
/// releases in the namespaces the cluster's tab watches to pick from).
pub fn install_tab(tab: &Entity<ClusterTab>, deps: &TabDeps, window: &mut Window, cx: &mut App) {
    let workspace = tab.read(cx).workspace().clone();
    let Some(panel) = workspace.read(cx).panel::<SidebarPanel>() else {
        tracing::warn!("the cluster tab has no sidebar: its Helm upgrade entry does not navigate");
        return;
    };
    let dispatcher = deps.dispatcher.clone();
    window
        .subscribe(&panel, cx, move |_, event: &SidebarEvent, _, cx| {
            if matches!(
                event,
                SidebarEvent::Navigate(SidebarTarget::Command(id)) if *id == CommandId::HELM_UPGRADE_RELEASE
            ) {
                dispatcher.dispatch(
                    Command::HelmUpgradeRelease {
                        namespace: None,
                        release: None,
                    },
                    cx,
                );
            }
        })
        .detach();
}
