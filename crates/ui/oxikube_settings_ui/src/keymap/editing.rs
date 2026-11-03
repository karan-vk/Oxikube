//! The keymap editor page's edits (E21-S08): open the key recorder on the selected row, and
//! remove the selected binding.

use gpui::{AppContext as _, Context, Entity, Window};
use oxikube_domain::command::Command;
use oxikube_keymap::edit::ref_of_row;
use oxikube_keymap::{KeymapPlatform, KeymapRow, RowStatus};

use super::recorder::KeybindRecorder;
use super::view::KeymapEditorView;

impl KeymapEditorView {
    /// The selected row, when one is showing.
    pub fn selected_row(&self, cx: &gpui::App) -> Option<KeymapRow> {
        self.table.read(cx, |delegate| delegate.selected().cloned())
    }

    /// Whether the selected row is a binding Remove can take out of force: one in force (a
    /// replaced or already unbound one is out of force; a command bound nowhere has nothing).
    pub fn can_remove_selected(&self, cx: &gpui::App) -> bool {
        self.selected_row(cx)
            .is_some_and(|row| row.status == RowStatus::Active && ref_of_row(&row).is_some())
    }

    /// Open the key recorder on the selected row (`keymap::RecordBinding`), in the workspace's
    /// modal layer. Returns the recorder, `None` without a selection or a workspace.
    pub fn record_selected(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<KeybindRecorder>> {
        let row = self.selected_row(cx)?;
        let workspace = self.deps.workspace.as_ref()?.upgrade()?;
        let dispatcher = self.deps.dispatcher.clone();
        let recorder = cx.new(|cx| {
            KeybindRecorder::new(&row, KeymapPlatform::current(), dispatcher, window, cx)
        });
        workspace.update(cx, |workspace, cx| {
            workspace.show_modal(recorder.clone(), window, cx);
        });
        self.recorder = Some(recorder.downgrade());
        Some(recorder)
    }

    /// The key recorder opened from the page, while it is open.
    pub fn recorder(&self) -> Option<Entity<KeybindRecorder>> {
        self.recorder.as_ref()?.upgrade()
    }

    /// Remove the selected binding (`keymap::RemoveBinding`): delete it from the user's file, or
    /// unbind a default one with `null` there. Returns whether a command was sent.
    pub fn remove_selected(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.can_remove_selected(cx) {
            return false;
        }
        let Some(binding) = self.selected_row(cx).as_ref().and_then(ref_of_row) else {
            return false;
        };
        self.deps
            .dispatcher
            .dispatch(Command::KeymapRemoveBinding { binding }, cx);
        true
    }
}
