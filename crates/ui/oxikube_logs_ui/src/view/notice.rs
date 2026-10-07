//! What the view tells the user about local actions (copied, saved, cleared): toasts in the
//! view's workspace.

use gpui::Context;
use oxikube_workspace::Toast;

use super::LogView;

impl LogView {
    /// Shows `toast` in the workspace the view is a tab of; nothing when it has none (a view
    /// built outside a workspace has nobody to tell).
    pub(crate) fn toast(&self, toast: Toast, cx: &mut Context<Self>) {
        if let Some(workspace) = self.workspace.as_ref() {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.show_toast(toast, cx);
                })
                .ok();
        }
    }
}
