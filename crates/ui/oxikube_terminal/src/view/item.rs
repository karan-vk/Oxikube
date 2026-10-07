//! The terminal as a workspace [`Item`]: its tab (title = process or pod name, dirty = a process
//! runs, icon by backend kind, the cluster's mark), docking, the split copy, closing (the process
//! ends), and persistence (the [`BackendDescriptor`] only).

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Window};
use oxikube_workspace::{Item, ItemEvent, TabContent, register_item};

use super::TerminalView;
use super::descriptor::BackendDescriptor;
use super::services::TerminalServices;

/// The kind terminal tabs are saved under (layout persistence, reopen-closed).
pub const TERMINAL_ITEM_KIND: &str = "terminal";

impl EventEmitter<ItemEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Item for TerminalView {
    fn tab_content(&self, cx: &App) -> TabContent {
        let content = TabContent::new(self.title())
            .icon(self.descriptor.icon())
            .dirty(self.is_running(cx));
        match self.mark {
            Some(mark) => content.cluster(mark),
            None => content,
        }
    }

    fn can_dock(&self, _: &App) -> bool {
        true
    }

    fn on_close(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.shut_down(cx);
    }

    /// A local shell splits into a fresh shell in the same directory. A pod terminal does not
    /// clone: another session in the container is a new `pod::Shell`, which the guard checks and
    /// audits (the controller's split sends it).
    fn clone_on_split(&self, _: &mut Window, cx: &mut Context<Self>) -> Option<Entity<Self>> {
        self.descriptor.is_local().then(|| self.duplicate(cx))
    }

    fn serialized_kind() -> Option<&'static str> {
        Some(TERMINAL_ITEM_KIND)
    }

    /// The descriptor only (with the shell's directory now): never the scrollback, the title the
    /// process set or the environment. A pod terminal is not saved: restoring it would open a
    /// session in the container at launch with nobody asking, around the guard's read-only block
    /// and audit; `pod: shell` opens a new one.
    fn serialize(&self, cx: &App) -> Option<serde_json::Value> {
        self.descriptor
            .is_local()
            .then(|| self.live_descriptor(cx).to_state())
    }
}

/// Registers how a saved terminal tab is rebuilt: a fresh process from its descriptor, through
/// the app's [`TerminalServices`]. A tab saved by another version, or restored before the
/// services are installed, is skipped.
pub(super) fn register_builder(cx: &mut App) {
    register_item::<TerminalView>(cx, |state, _, cx| {
        let descriptor = BackendDescriptor::from_state(state)?;
        let services = TerminalServices::try_global(cx)?;
        Some(cx.new(|cx| TerminalView::new(descriptor, services, cx)))
    });
}
