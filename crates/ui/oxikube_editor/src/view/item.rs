//! The manifest editor as a workspace [`Item`]: its tab (title, file icon, a dot once the buffer
//! changed) and what it never does: it is not saved with the layout and not restored, because a
//! manifest can hold Secret data and buffer contents never go to disk (non-negotiable 5).

use gpui::{App, EventEmitter, FocusHandle, Focusable};
use oxikube_ui::IconName;
use oxikube_workspace::{Item, ItemEvent, TabContent};

use super::manifest_editor::ManifestEditor;

impl EventEmitter<ItemEvent> for ManifestEditor {}

/// The buffer's own handle: focusing the tab puts the cursor in the text.
impl Focusable for ManifestEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Item for ManifestEditor {
    fn tab_content(&self, cx: &App) -> TabContent {
        TabContent::new(self.title.clone())
            .icon(IconName::FileCode)
            .dirty(self.is_dirty(cx))
    }

    // `serialized_kind` and `serialize` keep their `None` defaults: see the module docs.
}
