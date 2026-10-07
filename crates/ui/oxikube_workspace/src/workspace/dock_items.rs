//! Items in docks: opening a [dockable](crate::Item::can_dock) item in a dock, moving one there,
//! and asking which dock holds an item (Zed's terminal, which lives in the panes and the bottom
//! dock alike).
//!
//! A dock exists once a side panel was added to it ([`Workspace::add_panel`]): the panel is the
//! dock's anchor, so the dock never empties when its last item is dragged out or closed (the dock
//! area keeps a dock's last tab in place). Items share the tab group of the dock's first group
//! with its panels.

use gpui::{App, Context, EntityId, Window};
use oxikube_ui::dock::{InsertTarget, panel_handle};

use super::{Workspace, layout::first_group};
use crate::{item::ItemHandle, panel::DockPosition};

impl Workspace {
    /// Opens `item` in the dock at `position`, displays it there (opening the dock when it is
    /// closed) and focuses it when `focus`. An item that is open already moves there instead.
    ///
    /// Returns the item's id, or `None` when nothing opened: the item cannot
    /// [dock](crate::Item::can_dock), or there is no dock at `position` yet (add a panel there
    /// first). The item is dropped then.
    pub fn open_item_in_dock(
        &mut self,
        item: Box<dyn ItemHandle>,
        position: DockPosition,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<EntityId> {
        let item_id = item.item_id();
        if self.items.contains_key(&item_id) {
            return self
                .move_item_to_dock(item_id, position, window, cx)
                .then_some(item_id);
        }
        let placement = position.placement();
        if !item.can_dock(cx) || !self.dock_area.read(cx).has_dock(placement) {
            return None;
        }
        let tab = self.register_item(item, window, cx);
        self.dock_area.update(cx, |area, cx| {
            // Appended to the dock's first tab group and displayed there.
            area.add_panel_view(panel_handle(tab), placement, None, window, cx);
        });
        self.show_dock_item(item_id, position, focus, window, cx);
        Some(item_id)
    }

    /// Moves the open, [dockable](crate::Item::can_dock) `item` (from a pane or another dock) to
    /// the end of the first tab group of the dock at `position`, displays it there, opens the
    /// dock and focuses the item. A pane left empty disappears. Returns whether it moved (or was
    /// there already).
    pub fn move_item_to_dock(
        &mut self,
        item: EntityId,
        position: DockPosition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(open) = self.items.get(&item) else {
            return false;
        };
        if !open.handle.can_dock(cx) {
            return false;
        }
        let panel = open.panel_id;
        let placement = position.placement();
        let node = self
            .dock_area
            .read(cx)
            .layout(placement)
            .and_then(|tree| first_group(tree.root()))
            .map(|(node, _)| node);
        let Some(node) = node else {
            return false;
        };
        if self.item_dock(item, cx) != Some(position) {
            let target = InsertTarget::Tabs {
                node,
                ix: None,
                activate: true,
            };
            self.dock_area
                .update(cx, |area, cx| area.move_panel(panel, target, window, cx));
        }
        self.show_dock_item(item, position, true, window, cx);
        true
    }

    /// The dock holding `item`; `None` when it is in a centre pane or not open here.
    pub fn item_dock(&self, item: EntityId, cx: &App) -> Option<DockPosition> {
        let panel = self.items.get(&item)?.panel_id;
        DockPosition::from_placement(self.placement_of(panel, cx)?)
    }

    /// The items in the dock at `position`, in tab order.
    pub(super) fn dock_items(&self, position: DockPosition, cx: &App) -> Vec<EntityId> {
        let area = self.dock_area.read(cx);
        let Some(tree) = area.layout(position.placement()) else {
            return Vec::new();
        };
        tree.panels()
            .filter_map(|panel| self.item_panels.get(&panel).copied())
            .collect()
    }

    /// The item displayed by the first group of the dock at `position`, if an item is.
    pub(super) fn active_dock_item(&self, position: DockPosition, cx: &App) -> Option<EntityId> {
        let tree = self.dock_area.read(cx).layout(position.placement())?;
        let (_, displayed) = first_group(tree.root())?;
        self.item_panels.get(&displayed?).copied()
    }

    /// Opens the dock if needed, displays `item` in it and focuses it when `focus`.
    fn show_dock_item(
        &mut self,
        item: EntityId,
        position: DockPosition,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.items.get(&item).map(|open| open.panel_id) else {
            return;
        };
        let placement = position.placement();
        self.dock_area.update(cx, |area, cx| {
            if !area.is_dock_open(placement) {
                area.toggle_dock(placement, window, cx);
            }
            area.select_panel(panel, window, cx);
        });
        if focus {
            self.focus_item(item, window, cx);
        }
        cx.notify();
    }
}
