//! `oxikube_workspace` — layer: `ui`.
//!
//! Window shell: Item/Panel/Pane/Dock model, tabs, status bar, modal/toast layers, layout persistence, cluster tabs, sidebar, notifications panel.
//!
//! Module map:
//! - [`window`]: the main window (E05-S03): platform window options (native title bar on macOS,
//!   client-side decorations and the Wayland app id on Linux), the `Root` that hosts the overlay
//!   layers, the title bar, and the application menu.
//! - [`workspace`]: the [`Workspace`] entity (E05-S04): centre panes of items and side panels in
//!   docks, on gpui-component's dock area (through `oxikube_ui::dock`). API: `open_item`,
//!   `close_item`, `reopen_closed_item`, `split_pane`, `move_item`, `active_pane`,
//!   `add_panel`, `toggle_panel`, `toggle_dock`, `toggle_zoom`.
//! - [`item`]: the [`Item`] trait (tab content), [`ItemHandle`], the [`ItemRegistry`] that rebuilds
//!   closed items.
//! - [`panel`]: the [`Panel`] trait (dockable side panel), [`PanelHandle`], [`DockPosition`].
//! - [`pane`]: [`PaneGroup`] / [`Pane`] snapshots of the centre splits, [`SplitDirection`].
//! - [`dock`]: [`Dock`] snapshots of the edge docks.
//! - [`closed`]: the bounded reopen-closed stack.
//! - [`actions`]: `workspace::*` actions and their default key bindings.
//! - [`session`]: window and session basics (E05-S12): `window::New`, UI zoom (`view::ZoomIn`,
//!   `view::ZoomOut`, `view::ZoomReset`), reduce-motion, and the quit confirmation while
//!   operations run (`app::Quit`).
//!
//! The Item / Panel / Pane / Dock model follows Zed's `workspace` crate design; it is written
//! from scratch (no Zed code copied), on top of gpui-component's `DockArea`.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod actions;
pub mod closed;
pub mod dock;
pub mod item;
pub mod pane;
pub mod panel;
pub mod session;
mod tab_label;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod window;
pub mod workspace;

pub use closed::{ClosedItem, ClosedItemStack};
pub use dock::Dock;
pub use item::{Item, ItemEvent, ItemHandle, ItemRegistry, TabContent, register_item};
pub use pane::{Member, Pane, PaneAxis, PaneGroup, PaneId, SplitDirection};
pub use panel::{DockPosition, Panel, PanelEvent, PanelHandle};
pub use workspace::{OpenOptions, Workspace, WorkspaceEvent};

/// Registers the workspace: the main window's menu and actions ([`window::init`]), the
/// `workspace::*` key bindings and the session basics ([`session::init`]). Call once, after `oxikube_ui::init`.
pub fn init(cx: &mut gpui::App) {
    window::init(cx);
    actions::register(cx);
    session::init(cx);
}
