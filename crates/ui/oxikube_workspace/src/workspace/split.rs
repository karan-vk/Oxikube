//! Moving items between panes and splitting panes.

use gpui::{Context, EntityId, Window};
use oxikube_ui::dock::InsertTarget;

use super::{Workspace, open::Placement};
use crate::pane::{PaneId, SplitDirection};

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
                let id =
                    self.insert_item(clone, Placement::Split(pane, direction), true, window, cx);
                self.pane_group(cx).pane_for_item(id).map(|pane| pane.id())
            }
            None => self.move_item_to_split(item, direction, window, cx),
        }
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
}
