//! The sidebar's Helm entry: opens the Helm page in the cluster tab.

use std::rc::Rc;

use gpui::{App, AppContext as _, Entity, Window};
use oxikube_app::ClusterSession;
use oxikube_helm_ui::{HelmGateState, HelmPage};
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, CommandDispatcher, OpenOptions};

/// The page id the sidebar's Helm entry navigates to.
pub const HELM_PAGE: &str = "helm-releases";

/// Opens the [`HelmPage`] in `tab`'s workspace when the sidebar navigates to the Helm entry.
pub fn install_tab(
    tab: &Entity<ClusterTab>,
    session: &ClusterSession,
    dispatcher: &Rc<dyn CommandDispatcher>,
    window: &mut Window,
    cx: &mut App,
) {
    let workspace = tab.read(cx).workspace().clone();
    let Some(panel) = workspace.read(cx).panel::<SidebarPanel>() else {
        tracing::warn!(cluster = %session.id(), "the cluster tab has no sidebar: its Helm entry does not navigate");
        return;
    };
    let dispatcher = dispatcher.clone();
    window
        .subscribe(&panel, cx, move |_, event: &SidebarEvent, window, cx| {
            let SidebarEvent::Navigate(SidebarTarget::Page(page)) = event else {
                return;
            };
            if page.as_ref() != HELM_PAGE {
                return;
            }
            let state = HelmGateState::install(cx);
            let page = cx.new(|cx| HelmPage::new(state, dispatcher.clone(), cx));
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(page), OpenOptions::default(), window, cx);
            });
        })
        .detach();
}
