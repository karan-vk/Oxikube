//! Layout persistence (E05-S05): saving the workspace's layout and restoring it on launch.
//!
//! - [`model`]: [`SerializedWorkspace`], the versioned on-disk layout (gpui-component's
//!   `DockAreaState` with the open item descriptors, the active pane and the window place).
//! - [`store`]: [`LayoutStore`], one window's layout in the `StatePort` (SQLite in the app).
//! - [`controller`]: [`LayoutPersistence`], the entity that restores at start, debounces saves
//!   and flushes on quit.
//! - [`bounds`]: [`restore_window_bounds`], fitting saved window bounds to today's displays.
//! - [`prune`]: pure edits of saved `PanelState` trees.
//! - [`report`]: [`RestoreReport`], what a restore did.
//!
//! The workspace side (`Workspace::serialize_layout` / `restore_layout`) lives with the
//! workspace in `workspace::restore`. Items are rebuilt through the
//! [`ItemRegistry`](crate::ItemRegistry), keyed by their `serialized_kind`; kinds nobody
//! registered are skipped. Never put secrets in an item's or panel's serialised state.
//!
//! Order at launch: register item builders and add the side panels, open the state store (async),
//! then [`LayoutPersistence::start`]. The window may open at default bounds before the layout is
//! read; to open at the saved bounds instead, read the layout first
//! ([`LayoutStore::load`]) and pass its window through [`restore_window_bounds`] (see
//! [`crate::window::main_window_options_for`]).

mod bounds;
mod controller;
mod model;
mod prune;
mod report;
mod store;

pub use bounds::restore_window_bounds;
pub use controller::{LayoutPersistence, PersistenceEvent, RestoreStatus, SAVE_DEBOUNCE};
pub use model::{
    LAYOUT_SCHEMA_VERSION, LAYOUT_TABLE, LayoutError, MAIN_WINDOW_ID, SerializedWindow,
    SerializedWorkspace, WindowMode,
};
pub(crate) use prune::{item_descriptor, prune, surviving_active};
pub use report::{RestoreReport, SkipReason, SkippedItem};
pub use store::{LayoutStore, LoadOutcome};

#[cfg(test)]
mod tests;
