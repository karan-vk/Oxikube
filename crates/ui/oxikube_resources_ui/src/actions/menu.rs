//! The row actions in a row's context menu.

use gpui::{App, Context};
use oxikube_app::store::ObjectKey;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::kinds::ResourceKind;
use oxikube_ui::menu::{PopupMenu, PopupMenuItem};

use super::host::{ActionEntry, ResourceActions};
use crate::table::ResourceTable;

/// What a table's delegate needs to build the action entries of a menu: the shared actions, and
/// the cluster and kind of its rows.
#[derive(Clone)]
pub(crate) struct ActionSource {
    /// The shared actions.
    pub(crate) actions: ResourceActions,
    /// The table's cluster.
    pub(crate) cluster: ClusterId,
    /// The table's kind.
    pub(crate) kind: ResourceKind,
}

impl ActionSource {
    /// The `ResourceRef` of the object `key` in this table.
    pub(crate) fn target(&self, key: ObjectKey) -> ResourceRef {
        ResourceRef::new(
            self.cluster.clone(),
            self.kind.gvk.clone(),
            key.namespace,
            key.name,
        )
    }

    /// Appends the actions for `targets` to `menu`, after a separator. Choosing one runs it on
    /// `view` with these targets (the objects, not their rows: the feed moves rows while the menu
    /// is open). A disabled action is shown greyed out with its reason as a line under it.
    pub(crate) fn append(
        &self,
        menu: PopupMenu,
        targets: Vec<ResourceRef>,
        view: gpui::WeakEntity<ResourceTable>,
    ) -> PopupMenu {
        let entries = self
            .actions
            .entries(&self.cluster, &self.kind, targets.len());
        if entries.is_empty() {
            return menu;
        }
        let mut menu = menu.separator();
        for entry in entries {
            menu = add_entry(menu, entry, &targets, &view);
        }
        menu
    }
}

fn add_entry(
    menu: PopupMenu,
    entry: ActionEntry,
    targets: &[ResourceRef],
    view: &gpui::WeakEntity<ResourceTable>,
) -> PopupMenu {
    entry_items(&entry, targets, view)
        .into_iter()
        .fold(menu, PopupMenu::item)
}

/// The menu items of one action: the item itself (greyed out and not selectable when the action
/// is disabled), then, for a disabled action, a label line with the reason.
pub(super) fn entry_items(
    entry: &ActionEntry,
    targets: &[ResourceRef],
    view: &gpui::WeakEntity<ResourceTable>,
) -> Vec<PopupMenuItem> {
    let command = entry.command();
    let (view, targets) = (view.clone(), targets.to_vec());
    let item = PopupMenuItem::new(entry.label.clone())
        .disabled(!entry.is_enabled())
        .on_click(move |_, window, cx: &mut App| {
            view.update(cx, |table, cx: &mut Context<ResourceTable>| {
                table.run_action(command, targets.clone(), window, cx)
            })
            .ok();
        });
    let mut items = vec![item];
    items.extend(entry.reason().map(PopupMenuItem::label));
    items
}
