//! More than one window: `window::New` and the per-window close guard.
//!
//! Every main window has its own [`MainView`](crate::window::MainView), so its own `Workspace`
//! (panes, docks, tabs); what they share is app-wide state in GPUI globals (settings, theme
//! tokens, zoom, the quit guard). Opening a window therefore builds views only: it re-runs no
//! initialisation and reads nothing from disk.

use anyhow::Result;
use gpui::{App, Bounds, Pixels, Window, WindowBounds, WindowHandle, actions, point, px};
use oxikube_ui::root::Root;

use super::quit;
use crate::window;

actions!(
    window,
    [
        /// Opens another main window.
        New,
    ]
);

/// How far a new window is offset from the one it opens next to, so the two do not hide each
/// other exactly.
pub(super) const CASCADE: Pixels = px(28.);

/// Every open main window.
pub fn main_windows(cx: &App) -> Vec<WindowHandle<Root>> {
    cx.windows()
        .into_iter()
        .filter_map(|window| window.downcast::<Root>())
        .collect()
}

/// Opens another main window with a fresh, empty `Workspace`, offset from the active window.
pub fn open_new_window(cx: &mut App) -> Result<WindowHandle<Root>> {
    let bounds = cascaded_bounds(cx);
    window::open_main_window_at(cx, bounds, |content, _| content)
}

/// The active window's bounds moved down and right, when it is a plain (not maximised or
/// full-screen) window; `None` lets the new window be centred.
fn cascaded_bounds(cx: &mut App) -> Option<Bounds<Pixels>> {
    let active = cx.active_window()?;
    let bounds = active
        .update(cx, |_, window, _| match window.window_bounds() {
            WindowBounds::Windowed(bounds) => Some(bounds),
            _ => None,
        })
        .ok()??;
    Some(Bounds::new(
        bounds.origin + point(CASCADE, CASCADE),
        bounds.size,
    ))
}

/// Whether closing the last window quits the app: on Linux and Windows it does, on macOS the app
/// stays alive in the Dock (GPUI's default `QuitMode`, which does the quitting).
pub(super) fn last_window_close_quits() -> bool {
    !cfg!(target_os = "macos")
}

/// Whether a window may close without asking: always, unless it is the last one, closing it
/// quits, and a quit would stop running operations.
pub(super) fn allow_close(
    other_windows: bool,
    last_close_quits: bool,
    needs_confirmation: bool,
) -> bool {
    other_windows || !last_close_quits || !needs_confirmation
}

/// Installs the close guard on `window`: closing it asks first when [`allow_close`] says so.
pub(crate) fn install_close_guard(window: &Window, cx: &App) {
    window.on_window_should_close(cx, |window, cx| {
        should_close(window, cx, last_window_close_quits())
    });
}

/// The window's answer to "may I close?". When not, it opens the quit dialog on the window.
pub(super) fn should_close(window: &mut Window, cx: &mut App, last_close_quits: bool) -> bool {
    let other_windows = cx.windows().len() > 1;
    if allow_close(
        other_windows,
        last_close_quits,
        quit::needs_confirmation(cx),
    ) {
        return true;
    }
    quit::show_quit_prompt(window, cx);
    false
}

/// Registers the `window::New` handler (the key binding is in the keymap files).
pub(super) fn register(cx: &mut App) {
    cx.on_action(|_: &New, cx| {
        // Opened after the dispatching window's update, which holds that window's borrow.
        cx.defer(|cx| {
            if let Err(error) = open_new_window(cx) {
                tracing::warn!(%error, "could not open a new window");
            }
        });
    });
}
