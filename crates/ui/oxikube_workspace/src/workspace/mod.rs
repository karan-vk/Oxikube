//! [`Workspace`]: the entity that owns the window's layout: centre panes of items, and side
//! panels in the left, bottom and right docks.
//!
//! The layout itself lives in gpui-component's `DockArea` (through `oxikube_ui::dock`): a tree of
//! splits and tab groups per region that normalises itself on every edit, draws the tab bars,
//! handles tab drag and drop and dock resizing. The workspace adds the Zed-style model on top:
//! [`Item`](crate::Item)s and [`Panel`](crate::Panel)s instead of raw dock panels, the active
//! pane, open-or-activate, split, move, close and reopen-closed, dock toggling and zoom.
//!
//! - `open`: open-or-activate, placing a new item's tab.
//! - `close`: closing items, the reopen-closed stack.
//! - `split`: moving items between panes, splitting panes, returning item tabs dropped on a dock.
//! - `panels`: side panels, toggling a panel or a dock.
//! - `docks`: dock snapshots and sizes, zoom.
//! - `layout`: queries on the dock area's layout trees.
//! - `restore`: capturing the layout for persistence and restoring a saved one (E05-S05).
//! - `render`: the view and its action handlers.

mod close;
mod docks;
mod layout;
mod open;
mod panels;
mod render;
mod restore;
mod split;

#[cfg(test)]
mod tests;

use std::{collections::HashMap, rc::Rc};

use gpui::{
    AnyEntity, App, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, Subscription,
    Window,
};
use oxikube_ui::dock::{DockArea, DockEvent, DockPlacement, DockSkin, PanelId, PanelStyle};

use crate::{
    closed::ClosedItemStack,
    item::{ItemHandle, ItemTab},
    pane::{Pane, PaneGroup, PaneId},
    panel::{DockPosition, PanelHandle},
};

pub use open::OpenOptions;

/// Version written into the dock area's layout dump (E05-S05 persists it).
pub const LAYOUT_VERSION: usize = 1;

/// What the workspace reports outward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceEvent {
    /// The layout changed (an item opened, closed or moved, a split, a dock toggled or resized).
    /// Fires on every edit; a subscriber that writes to disk should debounce.
    LayoutChanged,
}

/// An open item and the dock panel that carries it.
struct OpenItem {
    handle: Box<dyn ItemHandle>,
    tab: Entity<ItemTab>,
    panel_id: PanelId,
    _subscriptions: Vec<Subscription>,
}

/// A side panel and the dock panel that carries it.
struct DockedPanel {
    handle: Box<dyn PanelHandle>,
    panel_id: PanelId,
    position: DockPosition,
    _subscription: Subscription,
}

/// The window's layout model. See the [module docs](self).
pub struct Workspace {
    dock_area: Entity<DockArea>,
    _skin: Rc<DockSkin>,
    items: HashMap<EntityId, OpenItem>,
    /// Dock panel id of an item tab -> the item it carries.
    item_panels: HashMap<PanelId, EntityId>,
    /// Where each item was at the last layout change. Read when the dock removes a tab, which
    /// happens after the tree has already forgotten it.
    locations: HashMap<EntityId, (PaneId, usize)>,
    panels: Vec<DockedPanel>,
    active_pane: Option<PaneId>,
    closed: ClosedItemStack,
    focus_handle: FocusHandle,
    /// Entities that live and die with the workspace (the layout persistence controller).
    attached: Vec<AnyEntity>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Workspace {
    /// An empty workspace: no items, no panels, no docks.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (dock_area, skin) = DockSkin::dock_area("workspace", Some(LAYOUT_VERSION), window, cx);
        // Items get a real tab bar even when alone in their pane, with close buttons.
        skin.set_panel_style(PanelStyle::TabBar, cx);
        skin.set_close_button_visible(true, cx);
        let subscription = cx.subscribe_in(
            &dock_area,
            window,
            |this, _, event: &DockEvent, window, cx| {
                if let DockEvent::LayoutChanged = event {
                    this.layout_changed(window, cx);
                }
            },
        );
        Self {
            dock_area,
            _skin: skin,
            items: HashMap::new(),
            item_panels: HashMap::new(),
            locations: HashMap::new(),
            panels: Vec::new(),
            active_pane: None,
            closed: ClosedItemStack::default(),
            focus_handle: cx.focus_handle(),
            attached: Vec::new(),
            _subscriptions: vec![subscription],
        }
    }

    /// Makes the workspace keep `entity` alive for as long as it lives itself. For helpers that
    /// observe the workspace (layout persistence) and hold it only weakly, so there is no cycle
    /// and no one has to remember to store them.
    pub fn attach<T: 'static>(&mut self, entity: Entity<T>) {
        self.attached.push(entity.into_any());
    }

    /// The dock area the layout lives in.
    pub fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// A snapshot of the centre's split layout.
    pub fn pane_group(&self, cx: &App) -> PaneGroup {
        match self.dock_area.read(cx).layout(DockPlacement::Center) {
            Some(tree) => PaneGroup::from_tree(tree, &self.item_panels),
            None => PaneGroup::default(),
        }
    }

    /// Every centre pane, in layout order.
    pub fn panes(&self, cx: &App) -> Vec<Pane> {
        self.pane_group(cx).panes().into_iter().cloned().collect()
    }

    /// The pane that receives newly opened items and pane commands: the one whose item last had
    /// focus (or was last opened, split or moved into); the first pane once that one is gone.
    /// `None` when the centre is empty.
    pub fn active_pane(&self, cx: &App) -> Option<Pane> {
        let group = self.pane_group(cx);
        self.active_pane
            .and_then(|id| group.pane(id))
            .or_else(|| group.panes().into_iter().next())
            .cloned()
    }

    /// Makes `pane` the active pane and focuses its displayed item.
    pub fn activate_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.pane_group(cx).pane(pane).cloned() else {
            return;
        };
        self.active_pane = Some(pane.id());
        if let Some(item) = pane.active_item() {
            self.focus_item(item, window, cx);
        }
        cx.notify();
    }

    /// The displayed item of the active pane.
    pub fn active_item(&self, cx: &App) -> Option<Box<dyn ItemHandle>> {
        let item = self.active_pane(cx)?.active_item()?;
        self.item(item).map(|item| item.boxed_clone())
    }

    /// The open item with `id`.
    pub fn item(&self, id: EntityId) -> Option<&dyn ItemHandle> {
        self.items.get(&id).map(|open| open.handle.as_ref())
    }

    /// Every open item, in no particular order.
    pub fn items(&self) -> impl Iterator<Item = &dyn ItemHandle> {
        self.items.values().map(|open| open.handle.as_ref())
    }

    /// The open items of type `T`.
    pub fn items_of_type<T: crate::Item>(&self) -> Vec<Entity<T>> {
        self.items()
            .filter_map(|item| item.downcast::<T>())
            .collect()
    }

    /// The reopen-closed stack.
    pub fn closed_items(&self) -> &ClosedItemStack {
        &self.closed
    }

    /// Whether nothing is open: no item and no panel. The view then draws a plain background
    /// instead of an empty dock area.
    pub fn is_blank(&self) -> bool {
        self.items.is_empty() && self.panels.is_empty()
    }

    fn layout_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Before `locations` is rebuilt: it still says where a stray item was dragged from.
        self.return_items_from_docks(window, cx);
        let group = self.pane_group(cx);
        self.locations.clear();
        for pane in group.panes() {
            for (ix, item) in pane.items().iter().enumerate() {
                self.locations.insert(*item, (pane.id(), ix));
            }
        }
        if self.active_pane.is_some_and(|id| group.pane(id).is_none()) {
            self.active_pane = group.panes().first().map(|pane| pane.id());
        }
        self.enforce_min_dock_sizes(window, cx);
        cx.emit(WorkspaceEvent::LayoutChanged);
        cx.notify();
    }

    /// The side panel whose entity id is `id`.
    fn docked(&self, id: EntityId) -> Option<&DockedPanel> {
        self.panels.iter().find(|p| p.handle.panel_id() == id)
    }

    /// Makes the pane holding `item` the active pane.
    fn activate_pane_of(&mut self, item: EntityId, cx: &App) {
        if let Some(pane) = self.pane_group(cx).pane_for_item(item) {
            self.active_pane = Some(pane.id());
        }
    }

    fn toggle_dock_area(&self, placement: DockPlacement, window: &mut Window, cx: &mut App) {
        self.dock_area
            .update(cx, |area, cx| area.toggle_dock(placement, window, cx));
    }

    fn focus_item(&self, item: EntityId, window: &mut Window, cx: &mut App) {
        if let Some(open) = self.items.get(&item) {
            open.handle.focus_handle(cx).focus(window, cx);
        }
    }

    /// Focuses the active pane's displayed item, or the workspace itself when there is none.
    fn focus_active_item(&self, window: &mut Window, cx: &mut App) {
        match self.active_pane(cx).and_then(|pane| pane.active_item()) {
            Some(item) => self.focus_item(item, window, cx),
            None => self.focus_handle.focus(window, cx),
        }
    }
}
