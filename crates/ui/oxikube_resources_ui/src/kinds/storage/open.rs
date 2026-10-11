//! Opening the storage dialogs in a cluster tab's workspace.

use gpui::{App, AppContext as _, Entity, Window};
use oxikube_domain::ids::ResourceRef;
use oxikube_workspace::Workspace;

use super::dialogs::{DefaultClassDialog, ResizeDialog};
use super::flows::StorageFlows;

/// Opens the resize dialog for the claim `target` as a modal of `workspace`.
pub(crate) fn open_resize(
    workspace: &Entity<Workspace>,
    flows: &StorageFlows,
    target: ResourceRef,
    window: &mut Window,
    cx: &mut App,
) {
    let flow = flows.resize.clone();
    let host = workspace.downgrade();
    workspace.update(cx, |workspace, cx| {
        let dialog = cx.new(|cx| ResizeDialog::new(flow, target, host, window, cx));
        workspace.show_modal(dialog.clone(), window, cx);
        dialog.update(cx, |dialog, cx| dialog.focus_input(window, cx));
    });
}

/// Opens the set-default dialog for the class `target` as a modal of `workspace`.
pub(crate) fn open_default_class(
    workspace: &Entity<Workspace>,
    flows: &StorageFlows,
    target: ResourceRef,
    window: &mut Window,
    cx: &mut App,
) {
    let flow = flows.default_class.clone();
    let host = workspace.downgrade();
    workspace.update(cx, |workspace, cx| {
        let dialog = cx.new(|cx| DefaultClassDialog::new(flow, target, host, cx));
        workspace.show_modal(dialog.clone(), window, cx);
        dialog.update(cx, |dialog, cx| dialog.focus_dialog(window, cx));
    });
}
