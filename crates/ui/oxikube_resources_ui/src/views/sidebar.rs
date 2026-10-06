//! From the cluster sidebar to the tables: [`sidebar_navigation`], the cluster-tab setup hook
//! that turns the sidebar's `Navigate(Kind { group, resource })` into
//! [`ResourceViews::navigate`] (discovery, then `resource::OpenList`), and its "Definitions"
//! entry (`Navigate(Command(crd::OpenList))`, E07-S07) into that command on the bus.

use std::cell::OnceCell;
use std::rc::Rc;

use gpui::{App, Entity, WeakEntity, Window};
use oxikube_app::ClusterSession;
use oxikube_domain::command::{Command, CommandId};
use oxikube_workspace::ClusterTab;
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};

use super::controller::ResourceViews;

/// Where the window's [`ResourceViews`] will be. The cluster tabs (and so their setup hooks)
/// are built before the controller that opens tables in them, so the hook reads it from here
/// once it is set. Cheap to clone.
#[derive(Clone, Default)]
pub struct ResourceViewsSlot(Rc<OnceCell<WeakEntity<ResourceViews>>>);

impl ResourceViewsSlot {
    /// An empty slot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fills the slot. The first one stays.
    pub fn set(&self, views: &Entity<ResourceViews>) {
        if self.0.set(views.downgrade()).is_err() {
            tracing::warn!("the resource views were set twice");
        }
    }

    /// The controller, while it lives.
    pub fn get(&self) -> Option<Entity<ResourceViews>> {
        self.0.get().and_then(WeakEntity::upgrade)
    }
}

/// The setup hook that opens a kind's table when its sidebar entry is activated. Run it after
/// the sidebar's own hook (it looks the tab's `SidebarPanel` up).
pub fn sidebar_navigation(
    views: ResourceViewsSlot,
) -> impl Fn(&Entity<ClusterTab>, &ClusterSession, &mut Window, &mut App) + 'static {
    move |tab, session, _, cx| {
        let Some(panel) = tab.read(cx).workspace().read(cx).panel::<SidebarPanel>() else {
            tracing::warn!("no cluster sidebar in the tab: kinds cannot be opened from it");
            return;
        };
        let views = views.clone();
        let cluster = session.id().clone();
        cx.subscribe(&panel, move |_, event: &SidebarEvent, cx| {
            let SidebarEvent::Navigate(target) = event;
            let Some(views) = views.get() else {
                return;
            };
            match target {
                SidebarTarget::Kind { group, resource } => {
                    views.update(cx, |views, cx| {
                        views.navigate(&cluster, group, resource, cx)
                    });
                }
                // The Custom Resources section's "Definitions": the CRD list, as a command.
                SidebarTarget::Command(id) if *id == CommandId::CRD_OPEN_LIST => {
                    let command = Command::CrdOpenList {
                        cluster: cluster.clone(),
                    };
                    let dispatcher = views.read(cx).deps.table.dispatcher.clone();
                    dispatcher.dispatch(command, cx);
                }
                SidebarTarget::Command(_) | SidebarTarget::Page(_) => {}
            }
        })
        .detach();
    }
}
