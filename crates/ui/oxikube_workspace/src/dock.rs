//! [`Dock`]: a snapshot of one edge dock (left, bottom or right).
//!
//! Zed's `Dock` is an entity holding its panels. Here the dock area owns the docks (their layout
//! tree, open state and size), so a `Dock` is read from it by
//! [`Workspace::dock`](crate::Workspace::dock); open, close and resize through the workspace.

use gpui::EntityId;
use oxikube_ui::Unscaled;

use crate::panel::DockPosition;

/// One edge dock, as it is now.
#[derive(Clone, Debug, PartialEq)]
pub struct Dock {
    pub(crate) position: DockPosition,
    pub(crate) open: bool,
    pub(crate) size: Unscaled,
    pub(crate) panels: Vec<EntityId>,
    pub(crate) active_panel: Option<EntityId>,
    pub(crate) items: Vec<EntityId>,
    pub(crate) active_item: Option<EntityId>,
}

impl Dock {
    /// Which edge.
    pub fn position(&self) -> DockPosition {
        self.position
    }

    /// Whether the dock is shown.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Its size along its axis (width for left/right, height for bottom), unscaled: the value
    /// to persist.
    pub fn size(&self) -> Unscaled {
        self.size
    }

    /// The panels in the dock, in tab order.
    pub fn panels(&self) -> &[EntityId] {
        &self.panels
    }

    /// The panel displayed by the dock's first group.
    pub fn active_panel(&self) -> Option<EntityId> {
        self.active_panel
    }

    /// The [dockable](crate::Item::can_dock) items in the dock, in tab order.
    pub fn items(&self) -> &[EntityId] {
        &self.items
    }

    /// The item displayed by the dock's first group, when an item (not a panel) is displayed.
    pub fn active_item(&self) -> Option<EntityId> {
        self.active_item
    }
}
