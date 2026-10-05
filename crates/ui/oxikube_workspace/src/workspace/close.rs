//! Closing items and reopening closed ones.

use gpui::{Context, EntityId, Window};

use super::{OpenItem, OpenOptions, Workspace};
use crate::{
    closed::ClosedItem,
    item::{ItemHandle, ItemRegistry},
    pane::PaneId,
};

impl Workspace {
    /// Closes `item` unless it refuses ([`Item::can_close`](crate::Item::can_close)). Its descriptor goes on the
    /// reopen-closed stack when the item is serialisable; a pane left empty disappears. Returns
    /// whether the item was closed.
    pub fn close_item(
        &mut self,
        item: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(open) = self.items.get(&item) else {
            return false;
        };
        if !open.handle.can_close(cx) {
            return false;
        }
        let had_focus = open.handle.focus_handle(cx).contains_focused(window, cx);
        let location = self
            .pane_group(cx)
            .pane_for_item(item)
            .map(|pane| (pane.id(), pane.index_of(item).unwrap_or_default()));
        let Some(open) = self.forget_item(item) else {
            return false;
        };
        self.remember_closed(open.handle.as_ref(), location, cx);
        // The dock tells the tab it was removed, which calls `Item::on_close`.
        self.dock_area.update(cx, |area, cx| {
            area.remove_panel(open.tab.clone(), window, cx)
        });
        if had_focus {
            self.focus_active_item(window, cx);
        }
        cx.notify();
        true
    }

    /// Closes the active pane's displayed item. Returns whether an item was closed.
    pub fn close_active_item(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.active_pane(cx).and_then(|pane| pane.active_item()) {
            Some(item) => self.close_item(item, window, cx),
            None => false,
        }
    }

    /// Reopens the most recently closed item where it was (its pane and index, when the pane
    /// still exists), rebuilt through the [`ItemRegistry`]. Entries whose kind has no builder or
    /// whose state the builder declines are skipped. Returns the reopened item.
    pub fn reopen_closed_item(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<EntityId> {
        while let Some(closed) = self.closed.pop() {
            let Some(item) = ItemRegistry::build(closed.kind, &closed.state, window, cx) else {
                continue;
            };
            let options = OpenOptions {
                pane: closed.pane,
                index: closed.index,
                ..OpenOptions::default()
            };
            return Some(self.open_item_with(item, options, window, cx));
        }
        None
    }

    /// The dock removed an item's tab on its own (its tab-bar close button).
    pub(super) fn item_tab_removed(
        &mut self,
        item: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let location = self.locations.get(&item).copied();
        let Some(open) = self.forget_item(item) else {
            return;
        };
        let had_focus = open.handle.focus_handle(cx).contains_focused(window, cx);
        self.remember_closed(open.handle.as_ref(), location, cx);
        if had_focus {
            self.focus_active_item(window, cx);
        }
        cx.notify();
    }

    fn forget_item(&mut self, item: EntityId) -> Option<OpenItem> {
        let open = self.items.remove(&item)?;
        self.item_panels.remove(&open.panel_id);
        self.locations.remove(&item);
        Some(open)
    }

    fn remember_closed(
        &mut self,
        item: &dyn ItemHandle,
        location: Option<(PaneId, usize)>,
        cx: &gpui::App,
    ) {
        let (Some(kind), Some(state)) = (item.serialized_kind(), item.serialize(cx)) else {
            return;
        };
        self.closed.push(ClosedItem {
            kind,
            state,
            title: item.tab_content(cx).title,
            pane: location.map(|(pane, _)| pane),
            index: location.map(|(_, index)| index),
        });
    }
}
