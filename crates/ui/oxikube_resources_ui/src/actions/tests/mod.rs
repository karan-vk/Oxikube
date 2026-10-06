//! Tests of the row actions in a real table, over a real `CommandBus` and `MutationGuard` on
//! testkit fakes (the table fixture's window, sessions and stores; no cluster, no threads).

mod bulk;
mod delete;
mod menu;

use gpui::{Entity, Modifiers, MouseButton};
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_workspace::toast::ToastSummary;

use crate::actions::DeleteDialog;
use crate::table::tests::fixture::{Fixture, cluster};

/// The nodes kind as discovery serves it (cluster-scoped: deleting one takes the typed name).
pub(super) fn nodes_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "Node"),
        preferred: true,
        plural: "nodes".into(),
        singular: "node".into(),
        short_names: vec!["no".into()],
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
        namespaced: false,
    }
}

/// A kind the server cannot delete (events are read-only here).
pub(super) fn readonly_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "ComponentStatus"),
        preferred: true,
        plural: "componentstatuses".into(),
        singular: "componentstatus".into(),
        short_names: vec!["cs".into()],
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list"]),
        namespaced: false,
    }
}

/// The delete dialog open in the cluster tab's workspace, if any.
pub(super) fn dialog(f: &mut Fixture) -> Option<Entity<DeleteDialog>> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let workspace = tab.read(cx).workspace().clone();
        let layer = workspace.read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<DeleteDialog>()
    })
}

/// The toasts of the cluster tab's workspace.
pub(super) fn toasts(f: &mut Fixture) -> Vec<ToastSummary> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let Some(tab) = tabs.read(cx).tab(&cluster()).cloned() else {
            return Vec::new();
        };
        let workspace = tab.read(cx).workspace().clone();
        let layer = workspace.read(cx).toast_layer().clone();
        layer.read(cx).visible()
    })
}

/// Right-clicks row `row` and opens its context menu.
pub(super) fn right_click(f: &mut Fixture, row: usize) {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let at = f
        .vcx
        .debug_bounds(Box::leak(format!("cell-{row}-0").into_boxed_str()))
        .unwrap_or_else(|| panic!("row {row} was not laid out"))
        .center();
    f.vcx
        .simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    f.vcx
        .simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    f.settle();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Chooses the `entry`-th selectable item of the open menu with the keys.
pub(super) fn choose(f: &mut Fixture, entry: usize) {
    let keys = std::iter::repeat_n("down", entry + 1)
        .chain(["enter"])
        .collect::<Vec<_>>()
        .join(" ");
    f.vcx.simulate_keystrokes(&keys);
    f.settle();
}

/// Runs `f` on the dialog.
pub(super) fn with_dialog<R>(
    f: &mut Fixture,
    dialog: &Entity<DeleteDialog>,
    run: impl FnOnce(&mut DeleteDialog, &mut gpui::Window, &mut gpui::Context<DeleteDialog>) -> R,
) -> R {
    let dialog = dialog.clone();
    let out = f
        .vcx
        .update(|window, cx| dialog.update(cx, |dialog, cx| run(dialog, window, cx)));
    f.settle();
    out
}
