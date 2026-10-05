//! Opening items: open-or-activate, and placing a new item's tab in the dock area.

use gpui::{AppContext as _, Context, Entity, EntityId, SharedString, Window};
use oxikube_ui::dock::{DockPlacement, InsertTarget, PanelId, panel_handle};

use super::{OpenItem, Workspace, layout::first_group};
use crate::{
    item::{Item, ItemEvent, ItemHandle, ItemTab, ItemTabEvent},
    pane::{PaneId, SplitDirection},
};

/// How [`Workspace::open_item_with`] places an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenOptions {
    /// The pane to open in; the active pane when `None` or when the pane no longer exists.
    pub pane: Option<PaneId>,
    /// The tab index in that pane; after the last tab when `None`.
    pub index: Option<usize>,
    /// Focus the item once it is displayed.
    pub focus: bool,
    /// When an open item has the same [`Item::item_key`], activate it instead of adding the new
    /// one (which is dropped).
    pub reuse_existing: bool,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            pane: None,
            index: None,
            focus: true,
            reuse_existing: true,
        }
    }
}

/// Where a new item tab lands.
pub(super) enum Placement {
    InPane(Option<PaneId>, Option<usize>),
    Split(PaneId, SplitDirection),
}

impl Workspace {
    /// Opens `item` in the active pane (or a new pane when the centre is empty), displays and
    /// focuses it. An item already open, or one whose [`Item::item_key`] matches an open item, is
    /// activated instead of duplicated. Returns the id of the item now displayed.
    pub fn open_item<T: Item>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EntityId {
        self.open_item_with(Box::new(item), OpenOptions::default(), window, cx)
    }

    /// [`Self::open_item`] with explicit [`OpenOptions`].
    pub fn open_item_with(
        &mut self,
        item: Box<dyn ItemHandle>,
        options: OpenOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EntityId {
        let existing = if self.items.contains_key(&item.item_id()) {
            Some(item.item_id())
        } else if options.reuse_existing {
            item.item_key(cx)
                .and_then(|key| self.find_item_by_key(&key, cx))
        } else {
            None
        };
        if let Some(existing) = existing {
            self.activate_item(existing, options.focus, window, cx);
            return existing;
        }
        let pane = options
            .pane
            .filter(|pane| self.pane_group(cx).pane(*pane).is_some());
        self.insert_item(
            item,
            Placement::InPane(pane, options.index),
            options.focus,
            window,
            cx,
        )
    }

    /// The open item whose [`Item::item_key`] is `key`.
    pub fn find_item_by_key(&self, key: &SharedString, cx: &gpui::App) -> Option<EntityId> {
        self.items
            .values()
            .find(|open| open.handle.item_key(cx).as_ref() == Some(key))
            .map(|open| open.handle.item_id())
    }

    /// Displays `item` in its pane, makes that pane active, and optionally focuses it.
    pub fn activate_item(
        &mut self,
        item: EntityId,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.items.get(&item).map(|open| open.panel_id) else {
            return;
        };
        self.dock_area
            .update(cx, |area, cx| area.select_panel(panel, window, cx));
        if let Some(pane) = self.pane_group(cx).pane_for_item(item) {
            self.active_pane = Some(pane.id());
        }
        if focus {
            self.focus_item(item, window, cx);
        }
        cx.notify();
    }

    /// Registers `item` and puts its tab where `placement` says.
    pub(super) fn insert_item(
        &mut self,
        item: Box<dyn ItemHandle>,
        placement: Placement,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EntityId {
        let item_id = item.item_id();
        let tab = cx.new(|_| ItemTab::new(item.boxed_clone()));
        let panel_id = PanelId::from(tab.entity_id());
        let subscriptions = self.subscribe_to_item(item.as_ref(), &tab, window, cx);
        self.items.insert(
            item_id,
            OpenItem {
                handle: item,
                tab: tab.clone(),
                panel_id,
                _subscriptions: subscriptions,
            },
        );
        self.item_panels.insert(panel_id, item_id);

        let target = match placement {
            Placement::InPane(pane, index) => pane
                .or_else(|| self.active_pane(cx).map(|pane| pane.id()))
                .map(|pane| InsertTarget::Tabs {
                    node: pane.node(),
                    ix: index,
                    activate: true,
                }),
            Placement::Split(pane, direction) => Some(InsertTarget::Split {
                node: pane.node(),
                placement: direction.placement(),
                size: None,
            }),
        };
        self.dock_area.update(cx, |area, cx| {
            // A dock panel can only enter the area through `add_panel_view`, which appends it to
            // the region's first tab group and displays it. It is then moved to its real place,
            // and the first group gets back the tab it was displaying.
            let first = area
                .layout(DockPlacement::Center)
                .and_then(|tree| first_group(tree.root()));
            area.add_panel_view(panel_handle(tab), DockPlacement::Center, None, window, cx);
            let Some(target) = target else {
                return;
            };
            let into_first = matches!((target, first),
                (InsertTarget::Tabs { node, .. }, Some((first, _))) if node == first);
            if into_first && matches!(target, InsertTarget::Tabs { ix: None, .. }) {
                return;
            }
            area.move_panel(panel_id, target, window, cx);
            if !into_first && let Some((_, Some(displayed))) = first {
                area.select_panel(displayed, window, cx);
            }
        });

        if let Some(pane) = self.pane_group(cx).pane_for_item(item_id) {
            self.active_pane = Some(pane.id());
        }
        if focus {
            self.focus_item(item_id, window, cx);
        }
        cx.notify();
        item_id
    }

    fn subscribe_to_item(
        &mut self,
        item: &dyn ItemHandle,
        tab: &Entity<ItemTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Subscription> {
        let item_id = item.item_id();
        let workspace = cx.weak_entity();
        let events = item.subscribe_to_item_events(
            window,
            cx,
            Box::new(move |event, window, cx| {
                let event = *event;
                workspace
                    .update(cx, |this, cx| {
                        this.on_item_event(item_id, event, window, cx)
                    })
                    .ok();
            }),
        );
        let removed = cx.subscribe_in(tab, window, move |this, _, event, window, cx| match event {
            ItemTabEvent::Removed => this.item_tab_removed(item_id, window, cx),
        });
        let focused = cx.on_focus_in(&item.focus_handle(cx), window, move |this, _, cx| {
            if let Some(pane) = this.pane_group(cx).pane_for_item(item_id) {
                this.active_pane = Some(pane.id());
            }
        });
        vec![events, removed, focused]
    }

    fn on_item_event(
        &mut self,
        item: EntityId,
        event: ItemEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The tab bar reads the tab content while it renders: a redraw is enough.
            ItemEvent::UpdateTab => cx.notify(),
            ItemEvent::CloseItem => {
                self.close_item(item, window, cx);
            }
        }
    }
}
