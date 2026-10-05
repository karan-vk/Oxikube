//! The main window (E05-S03).
//!
//! - [`init`]: registers the application menu, its actions and key bindings.
//! - [`open_main_window`]: opens the themed window with the platform's title bar. The window
//!   root is `oxikube_ui`'s `Root`, which renders the dialog, sheet and notification layers
//!   exactly once above [`MainView`].
//! - [`options`]: the per-platform `WindowOptions` and the application id.
//! - [`menus`]: the macOS app menu and its actions.
//!
//! Order in the binary: build the `Application` with `oxikube_ui::Assets`, then in `run`:
//! `oxikube_ui::init(cx)`, [`init`], [`open_main_window`]. Nothing here reads the disk or the
//! network, so the first frame does not wait on I/O (docs/PERFORMANCE.md, cold start).

pub mod menus;
pub mod options;
mod view;

#[cfg(test)]
mod tests;

use anyhow::{Context as _, Result};
use gpui::{AnyView, App, AppContext as _, Entity, Window, WindowHandle};
use oxikube_ui::root::{Root, new_root};

pub use menus::{
    About, Hide, HideOthers, Minimize, OpenPreferences, Quit, ShowAll, Zoom, app_menus,
};
pub use options::{APP_ID, Chrome, WINDOW_TITLE, main_window_options, window_options};
pub use view::MainView;

/// Registers the application menu (`cx.set_menus`), its action handlers and key bindings.
/// Idempotent only in effect: call it once, after `oxikube_ui::init`.
pub fn init(cx: &mut App) {
    menus::register(cx);
}

/// Opens the main window and returns its handle. The window's root view is the [`Root`].
pub fn open_main_window(cx: &mut App) -> Result<WindowHandle<Root>> {
    open_main_window_with(cx, |content, _| content)
}

/// Like [`open_main_window`], with `wrap` applied to the content view before the `Root` hosts
/// it. The binary uses it to put the `--perf` frame hook between the `Root` and the content:
/// the `Root` must stay the window's root view or overlays stop working.
pub fn open_main_window_with(
    cx: &mut App,
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView + 'static,
) -> Result<WindowHandle<Root>> {
    let options = main_window_options(cx);
    cx.open_window(options, move |window, cx| build_root(window, cx, wrap))
        .context("opening the main window")
}

/// Builds the window root: [`MainView`] (wrapped by `wrap`) inside the `Root`. Also what the
/// headless screenshot renders, so the picture is of the real window content.
pub fn build_root(
    window: &mut Window,
    cx: &mut App,
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView,
) -> Entity<Root> {
    let content: AnyView = cx.new(|_| MainView::new()).into();
    let content = wrap(content, cx);
    new_root(content, window, cx)
}
