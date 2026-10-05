//! Workspace actions and their default key bindings (Zed's bindings, context `Workspace`).
//!
//! The ids follow the `namespace::Verb` command naming (`workspace::SplitRight`), so each maps
//! one to one onto a `Command` once the `CommandBus` and its MCP tool stubs land (E11), and the
//! bindings move into the keymap files of `oxikube_keymap` (E05-S07). Until then the workspace
//! view handles them directly (see `workspace::render`).

use gpui::{App, KeyBinding, actions};

/// The key context the workspace view sets.
pub const WORKSPACE_KEY_CONTEXT: &str = "Workspace";

actions!(
    workspace,
    [
        /// Split the active pane; the new pane goes to the left.
        SplitLeft,
        /// Split the active pane; the new pane goes to the right.
        SplitRight,
        /// Split the active pane; the new pane goes above.
        SplitUp,
        /// Split the active pane; the new pane goes below.
        SplitDown,
        /// Close the active pane's displayed item.
        CloseActiveItem,
        /// Reopen the most recently closed item.
        ReopenClosedItem,
        /// Open or close the left dock.
        ToggleLeftDock,
        /// Open or close the right dock.
        ToggleRightDock,
        /// Open or close the bottom dock.
        ToggleBottomDock,
        /// Zoom the active pane (or the focused dock group) in or out.
        ToggleZoom,
    ]
);

/// The default key bindings of the workspace actions.
pub fn default_bindings(macos: bool) -> Vec<KeyBinding> {
    let ctx = Some(WORKSPACE_KEY_CONTEXT);
    let m = if macos { "cmd" } else { "ctrl" };
    vec![
        KeyBinding::new(&format!("{m}-k left"), SplitLeft, ctx),
        KeyBinding::new(&format!("{m}-k right"), SplitRight, ctx),
        KeyBinding::new(&format!("{m}-k up"), SplitUp, ctx),
        KeyBinding::new(&format!("{m}-k down"), SplitDown, ctx),
        KeyBinding::new(&format!("{m}-w"), CloseActiveItem, ctx),
        KeyBinding::new(&format!("{m}-shift-t"), ReopenClosedItem, ctx),
        KeyBinding::new(&format!("{m}-b"), ToggleLeftDock, ctx),
        KeyBinding::new(
            if macos { "cmd-r" } else { "ctrl-alt-b" },
            ToggleRightDock,
            ctx,
        ),
        KeyBinding::new(&format!("{m}-j"), ToggleBottomDock, ctx),
        KeyBinding::new("shift-escape", ToggleZoom, ctx),
    ]
}

/// Registers the default key bindings. Called by [`crate::init`].
pub(crate) fn register(cx: &mut App) {
    cx.bind_keys(default_bindings(cfg!(target_os = "macos")));
}
