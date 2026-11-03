//! The keymap editor page of the main window (E21-S07): `keymap::ShowEditor` opens it in the
//! active cluster's tab, and the sidebar's Settings > Keymap entry asks for it.
//!
//! * [`start_views`] applies each `keymap::ShowEditor` / `keymap::RecordBinding` request on the UI
//!   thread (`oxikube_settings_ui::keymap::KeymapEditorViews`).
//! * [`install_tab`] runs for every cluster tab: a click on the sidebar's Keymap entry is the
//!   same command.

use std::rc::Rc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Task, WeakEntity, Window};
use oxikube_domain::command::{Command, CommandId};
use oxikube_settings_ui::keymap::{KeymapEditorRequest, KeymapEditorViews, KeymapEditorViewsDeps};
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, ClusterTabs, CommandDispatcher};

use super::tabs::TabDeps;

/// Starts the window's keymap editor page over `requests` (the bus's `keymap::ShowEditor` and
/// `keymap::RecordBinding` handlers). Keep the task for the life of the window.
pub fn start_views(
    dispatcher: Rc<dyn CommandDispatcher>,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<KeymapEditorRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    KeymapEditorViews::start(
        KeymapEditorViewsDeps { dispatcher, tabs },
        requests,
        window,
        cx,
    )
}

/// Routes a click on the sidebar's Keymap entry to `keymap::ShowEditor`.
pub fn install_tab(tab: &Entity<ClusterTab>, deps: &TabDeps, window: &mut Window, cx: &mut App) {
    let workspace = tab.read(cx).workspace().clone();
    let Some(panel) = workspace.read(cx).panel::<SidebarPanel>() else {
        tracing::warn!("the cluster tab has no sidebar: its Keymap entry does not navigate");
        return;
    };
    let dispatcher = deps.dispatcher.clone();
    window
        .subscribe(&panel, cx, move |_, event: &SidebarEvent, _, cx| {
            if matches!(
                event,
                SidebarEvent::Navigate(SidebarTarget::Command(id)) if *id == CommandId::KEYMAP_SHOW_EDITOR
            ) {
                dispatcher.dispatch(Command::KeymapShowEditor, cx);
            }
        })
        .detach();
}
