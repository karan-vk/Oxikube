//! The sidebar's GPUI actions and key bindings.
//!
//! View-level navigation only (move, expand, collapse, activate): none changes a cluster, and
//! the toggle command of the sidebar itself (`sidebar.toggle`) belongs with the palette work
//! (E11), so the panel's toggle action is declared here without a key binding.

use gpui::{App, KeyBinding, NoAction, actions};

/// Key context of the focused sidebar list.
pub const SIDEBAR_CONTEXT: &str = "ClusterSidebar";

/// Key context of the Custom Resources search field, nested in [`SIDEBAR_CONTEXT`]. The text
/// input binds no `space`, so without this context the sidebar's `space` (Activate) would take
/// the keystroke and a space could never be typed into the search.
pub const SIDEBAR_SEARCH_CONTEXT: &str = "ClusterSidebarSearch";

actions!(
    cluster_sidebar,
    [
        /// Opens or closes the cluster sidebar (the panel's toggle action; no key yet).
        Toggle,
        /// Highlights the previous row.
        MoveUp,
        /// Highlights the next row.
        MoveDown,
        /// Opens the highlighted group, or goes to the highlighted entry.
        Activate,
        /// Closes the highlighted group.
        Collapse,
        /// Opens the highlighted group.
        Expand,
    ]
);

/// Binds the sidebar's keys. Called by [`super::init`].
pub(super) fn register(cx: &mut App) {
    let ctx = Some(SIDEBAR_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("up", MoveUp, ctx),
        KeyBinding::new("down", MoveDown, ctx),
        KeyBinding::new("enter", Activate, ctx),
        KeyBinding::new("space", Activate, ctx),
        KeyBinding::new("left", Collapse, ctx),
        KeyBinding::new("right", Expand, ctx),
        // In the search field a space is text: the deeper null binding masks Activate.
        KeyBinding::new("space", NoAction, Some(SIDEBAR_SEARCH_CONTEXT)),
    ]);
}
