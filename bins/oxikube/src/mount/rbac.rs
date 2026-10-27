//! The RBAC subject pages of the main window (E17-S07): `rbac::OpenUsers` (`:usr`), `rbac::OpenGroups`
//! (`:grp`) and `rbac::ShowSubject` (`:rbac <subject>`) open them in the cluster's tab, and
//! `rbac::CopyMatrix` copies the open matrix.
//!
//! * [`start_views`] applies each `rbac::*` request on the UI thread
//!   (`oxikube_rbac_ui::subjects::SubjectsViews`). A page reads the cluster's Roles and bindings
//!   from the cluster's `ResourceStore` (four shared feeds, all namespaces), found through the
//!   connected session each time the page opens, so a reconnect gets the new store.
//! * [`install_tab`] runs for every cluster tab: a click on the sidebar's Access Control > Users or
//!   Groups entry is the same command, with the tab's cluster.
//!
//! The palette's "RBAC: ..." commands and the `:usr` / `:grp` / `:rbac` jumps send the same
//! commands, so a user reaches the pages from the sidebar, the jump bar or the palette.

use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, WeakEntity, Window};
use oxikube_app::rbac_subjects::SubjectsService;
use oxikube_app::{ClusterSession, ClusterSessionManager, ResourceStores};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_rbac_ui::subjects::{
    Backends, RbacRequest, StoreBackend, StoreLookup, SubjectsViews, SubjectsViewsDeps,
};
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, ClusterTabs, CommandDispatcher};

use super::tabs::TabDeps;

/// Starts the window's subject pages over `requests` (the bus's `rbac::*` handlers). Keep the
/// entity for the life of the window.
pub fn start_views(
    service: SubjectsService,
    sessions: ClusterSessionManager,
    stores: Arc<ResourceStores>,
    dispatcher: Rc<dyn CommandDispatcher>,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<RbacRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SubjectsViews> {
    // The store of a connected session; the session's store is rebuilt after a reconnect.
    let lookup: StoreLookup = Rc::new(move |cluster: &ClusterId| {
        let session = sessions.get(cluster).filter(ClusterSession::is_connected)?;
        stores.for_session(&session)
    });
    let backends: Backends = Rc::new(move |cluster: &ClusterId| {
        Rc::new(StoreBackend::new(
            cluster.clone(),
            service.clone(),
            lookup.clone(),
        ))
    });
    let deps = SubjectsViewsDeps {
        backends,
        dispatcher,
        host: Rc::new(tabs),
    };
    SubjectsViews::start(deps, requests, window, cx)
}

/// Routes a click on the sidebar's Users and Groups entries to `rbac::OpenUsers` and
/// `rbac::OpenGroups` for this tab's cluster.
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
        tracing::warn!(%cluster, "the cluster tab has no sidebar: its RBAC entries do not navigate");
        return;
    };
    let dispatcher = deps.dispatcher.clone();
    window
        .subscribe(&panel, cx, move |_, event: &SidebarEvent, _, cx| {
            let SidebarEvent::Navigate(SidebarTarget::Command(id)) = event else {
                return;
            };
            let cluster = cluster.clone();
            let command = if *id == CommandId::RBAC_OPEN_USERS {
                Command::RbacOpenUsers { cluster }
            } else if *id == CommandId::RBAC_OPEN_GROUPS {
                Command::RbacOpenGroups { cluster }
            } else {
                return;
            };
            dispatcher.dispatch(command, cx);
        })
        .detach();
}
