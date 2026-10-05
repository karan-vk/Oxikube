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
//! - [`persistence`]: layout persistence (E05-S05): the versioned saved layout, the `StatePort`-backed
//!   store, the controller that restores at launch, debounces saves and flushes on quit, and window
//!   bounds fitted to the displays.
//! - [`closed`]: the bounded reopen-closed stack.
//! - [`actions`]: `workspace::*` actions and their default key bindings.
//! - [`session`]: window and session basics (E05-S12): `window::New`, UI zoom (`view::ZoomIn`,
//!   `view::ZoomOut`, `view::ZoomReset`), reduce-motion, and the quit confirmation while
//!   operations run (`app::Quit`).
//! - [`status_bar`]: the [`StatusBar`] with its left and right [`StatusItem`] registry (E05-S10).
//! - [`modal`]: the [`ModalLayer`]: one [`ModalView`] at a time, Escape and outside-click
//!   dismissal, Tab trapped inside, focus restored on close; [`DialogModal`] for confirmations.
//! - [`toast`]: the [`ToastLayer`]: queued, deduplicated, auto-dismissing [`Toast`]s with actions.
//! - [`motion`]: the reduce-motion switch and the 150 ms animation cap the layers follow.
//!
//! Features use the layers through the [`Workspace`]: `register_status_item`, `toggle_modal`,
//! `show_toast`.
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
pub mod modal;
pub mod motion;
pub mod pane;
pub mod panel;
pub mod persistence;
pub mod session;
pub mod status_bar;
mod tab_label;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod toast;
pub mod window;
pub mod workspace;

pub use closed::{ClosedItem, ClosedItemStack};
pub use dock::Dock;
pub use item::{Item, ItemEvent, ItemHandle, ItemRegistry, TabContent, register_item};
pub use modal::{DialogModal, ModalLayer, ModalView};
pub use pane::{Member, Pane, PaneAxis, PaneGroup, PaneId, SplitDirection};
pub use panel::{DockPosition, Panel, PanelEvent, PanelHandle};
pub use status_bar::{StatusBar, StatusItem, StatusItemId, StatusSide};
pub use toast::{Toast, ToastAction, ToastId, ToastLayer, ToastLevel};
pub use workspace::{OpenOptions, Workspace, WorkspaceEvent};

/// Registers the workspace: the main window's menu and actions ([`window::init`]), the
/// `workspace::*` key bindings, the session basics ([`session::init`]) and the modal and toast
/// layers' key bindings. Call once, after
/// `oxikube_ui::init`.
pub fn init(cx: &mut gpui::App) {
    window::init(cx);
    actions::register(cx);
    session::init(cx);
    modal::register(cx);
    toast::register(cx);
}
