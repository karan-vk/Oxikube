//! Moving items between panes and splitting panes; item tabs dropped on a dock go back.

use gpui::{Context, EntityId, Window};
use oxikube_ui::dock::{DockPlacement, InsertTarget, PanelId};

use super::{Workspace, layout::first_group, open::ItemPlacement};
use crate::{
    item::ItemHandle,
    pane::{PaneId, SplitDirection},
    panel::DockPosition,
};

impl Workspace {
    /// Moves `item` into `pane` at `index` (after the last tab when `None`), displays it there and
    /// makes that pane active. A pane left empty disappears. Returns whether the item moved.
    pub fn move_item(
        &mut self,
        item: EntityId,
        pane: PaneId,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(panel) = self.items.get(&item).map(|open| open.panel_id) else {
            return false;
        };
        if self.pane_group(cx).pane(pane).is_none() {
            return false;
        }
        let target = InsertTarget::Tabs {
            node: pane.node(),
            ix: index,
            activate: true,
        };
        self.dock_area
            .update(cx, |area, cx| area.move_panel(panel, target, window, cx));
        self.active_pane = Some(pane);
        self.focus_item(item, window, cx);
        cx.notify();
        true
    }

    /// Moves `item` out of its pane into a new pane beside it. Returns the new pane, or `None`
    /// when the item is the only one in its pane (there would be nothing left to split from).
    pub fn move_item_to_split(
        &mut self,
        item: EntityId,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        let panel = self.items.get(&item)?.panel_id;
        let pane = self.pane_group(cx).pane_for_item(item)?.clone();
        if pane.len() < 2 {
            return None;
        }
        let target = InsertTarget::Split {
            node: pane.id().node(),
            placement: direction.placement(),
            size: None,
        };
        self.dock_area
            .update(cx, |area, cx| area.move_panel(panel, target, window, cx));
        let new_pane = self.pane_group(cx).pane_for_item(item)?.id();
        self.active_pane = Some(new_pane);
        self.focus_item(item, window, cx);
        cx.notify();
        Some(new_pane)
    }

    /// Splits `pane` in `direction` (Zed's split): the new pane gets a copy of the displayed item
    /// when it supports [`Item::clone_on_split`](crate::Item::clone_on_split), otherwise the displayed item itself moves there
    /// if the pane has another item to keep. Returns the new pane, which becomes active.
    pub fn split_pane(
        &mut self,
        pane: PaneId,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        let item = self.pane_group(cx).pane(pane)?.active_item()?;
        let handle = self.items.get(&item)?.handle.boxed_clone();
        match handle.clone_on_split(window, cx) {
            Some(clone) => {
                let id = self.insert_item(
                    clone,
                    ItemPlacement::Split(pane, direction),
                    true,
                    window,
                    cx,
                );
                self.pane_group(cx).pane_for_item(id).map(|pane| pane.id())
            }
            None => self.move_item_to_split(item, direction, window, cx),
        }
    }

    /// Opens `item` in a new pane beside `pane` (the active pane when `None`), in `direction`,
    /// and displays and focuses it there. With no pane in the centre yet it opens as the first
    /// one. Returns the id of the item.
    pub fn open_item_in_split(
        &mut self,
        item: Box<dyn ItemHandle>,
        pane: Option<PaneId>,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EntityId {
        let pane = pane
            .filter(|pane| self.pane_group(cx).pane(*pane).is_some())
            .or_else(|| self.active_pane(cx).map(|pane| pane.id()));
        let placement = match pane {
            Some(pane) => ItemPlacement::Split(pane, direction),
            None => ItemPlacement::InPane(None, None),
        };
        self.insert_item(item, placement, true, window, cx)
    }

    /// [`Self::split_pane`] on the active pane.
    pub fn split_active_pane(
        &mut self,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        let pane = self.active_pane(cx)?.id();
        self.split_pane(pane, direction, window, cx)
    }

    /// Puts back into the centre every item tab that a drag left in a dock. Items live in centre
    /// panes (Zed's model) unless they [can dock](crate::Item::can_dock): the dock area accepts
    /// any tab on any tab bar, so the tab of any other item dropped on a dock's tab bar is
    /// returned to the pane and index it was dragged from, or to the active pane when that pane
    /// is gone, and displayed and focused there.
    pub(super) fn return_items_from_docks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let area = self.dock_area.read(cx);
        let stray: Vec<(EntityId, PanelId)> = DockPosition::ALL
            .iter()
            .filter_map(|position| area.layout(position.placement()))
            .flat_map(|tree| tree.panels())
            .filter_map(|panel| self.item_panels.get(&panel).map(|item| (*item, panel)))
            .filter(|(item, _)| {
                !self
                    .items
                    .get(item)
                    .is_some_and(|open| open.handle.can_dock(cx))
            })
            .collect();
        for (item, panel) in stray {
            let group = self.pane_group(cx);
            let (node, ix) = match self
                .locations
                .get(&item)
                .filter(|(pane, _)| group.pane(*pane).is_some())
            {
                Some((pane, ix)) => (pane.node(), Some(*ix)),
                None => {
                    let fallback =
                        self.active_pane(cx)
                            .map(|pane| pane.id().node())
                            .or_else(|| {
                                let area = self.dock_area.read(cx);
                                let tree = area.layout(DockPlacement::Center)?;
                                first_group(tree.root()).map(|(node, _)| node)
                            });
                    let Some(node) = fallback else {
                        continue;
                    };
                    (node, None)
                }
            };
            let target = InsertTarget::Tabs {
                node,
                ix,
                activate: true,
            };
            self.dock_area
                .update(cx, |area, cx| area.move_panel(panel, target, window, cx));
            self.activate_pane_of(item, cx);
            self.focus_item(item, window, cx);
        }
    }
}
