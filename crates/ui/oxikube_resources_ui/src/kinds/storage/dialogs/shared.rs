//! What the two storage dialogs share: turning a `spawn_kube` result into text, and toasts on the
//! workspace.

use std::fmt::Display;

use gpui::{AppContext, WeakEntity};
use oxikube_domain::OxiResult;
use oxikube_workspace::{Toast, Workspace};

/// A `spawn_kube` result as the value or the text the dialog shows when it failed: the call's own
/// message, or the task's error (it was aborted, the runtime is gone).
pub(super) fn settle<T, E: Display>(result: Result<OxiResult<T>, E>) -> Result<T, String> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.message().to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// Shows `toast` on `workspace`, if it is still there.
pub(super) fn show_toast<C: AppContext>(
    workspace: &WeakEntity<Workspace>,
    toast: Toast,
    cx: &mut C,
) {
    workspace
        .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
        .ok();
}
