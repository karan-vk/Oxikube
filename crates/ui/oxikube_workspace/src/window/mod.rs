//! The main window (E05-S03).
//!
//! - [`init`]: registers the application menu, its actions and key bindings.
//! - [`open_main_window`]: opens the themed window with the platform's title bar. The window
//!   root is `oxikube_ui`'s `Root`, which renders the dialog, sheet and notification layers
//!   exactly once above [`MainView`].
//! - [`open_main_window_restoring`]: the same window, restoring its saved layout in the
//!   background behind the startup placeholder (E05-S13; see [`MainView`]).
//! - [`open_main_window_mounted`]: the same, with a mount hook that fills the new window before
//!   its first frame (the binary puts the catalog home, the hotbar and the cluster tabs there,
//!   E07-S00). The app opens its first window this way.
//! - [`options`]: the per-platform `WindowOptions` and the application id.
//! - [`menus`]: the macOS app menu and its actions.
//!
//! Order in the binary: build the `Application` with `oxikube_ui::Assets`, then in `run`:
//! `oxikube_ui::init(cx)`, [`init`], [`open_main_window_restoring`]. Nothing here reads the disk
//! or the network on the UI thread (the layout is read through the async `StatePort`), so the
//! first frame does not wait on I/O (docs/PERFORMANCE.md, cold start).

pub mod menus;
pub mod options;
mod view;

#[cfg(test)]
mod restore_tests;
#[cfg(test)]
mod tests;

use anyhow::{Context as _, Result};
use gpui::{
    AnyView, App, AppContext as _, Bounds, Entity, Pixels, Window, WindowBounds, WindowHandle,
};
use oxikube_ui::root::{Root, new_root};

pub use crate::session::Quit;
pub use menus::{About, Hide, HideOthers, Minimize, OpenPreferences, ShowAll, Zoom, app_menus};
pub use options::{
    APP_ID, Chrome, WINDOW_TITLE, main_window_options, main_window_options_for, window_options,
};
pub use view::{MainView, RESTORING_LABEL};

use crate::persistence::LayoutStore;

/// Registers the application menu's action handlers and key bindings; the menu bar itself is
/// installed at the end of the first main window's first frame ([`menus::install_once`]).
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
    open_main_window_at(cx, None, wrap)
}

/// Like [`open_main_window_with`], at `bounds` (centred when `None`). Every main window gets
/// the close guard of the session module: the last one asks before quitting while operations
/// run (E05-S12).
pub fn open_main_window_at(
    cx: &mut App,
    bounds: Option<Bounds<Pixels>>,
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView + 'static,
) -> Result<WindowHandle<Root>> {
    open(cx, bounds, None, wrap)
}

/// Like [`open_main_window_with`], restoring the layout saved in `layout` and saving it from
/// then on. The window opens at once with the startup placeholder (the default layout, marked
/// "Restoring layout…") and the saved layout replaces it when the asynchronous read completes; a
/// failed read leaves the default layout in place and usable.
pub fn open_main_window_restoring(
    cx: &mut App,
    layout: LayoutStore,
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView + 'static,
) -> Result<WindowHandle<Root>> {
    open(cx, None, Some(layout), wrap)
}

/// Like [`open_main_window_restoring`] (or [`open_main_window_with`] when `layout` is `None`),
/// with `mount` run on the new [`MainView`] (its workspace and layout persistence) before the
/// `Root` hosts it, so what it opens (the catalog home, the hotbar) is in the first frame. Items
/// `mount` opens are kept by the layout restore, which only fills an empty centre.
pub fn open_main_window_mounted(
    cx: &mut App,
    layout: Option<LayoutStore>,
    mount: impl FnOnce(&Entity<MainView>, &mut Window, &mut App) + 'static,
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView + 'static,
) -> Result<WindowHandle<Root>> {
    open_with(cx, None, layout, mount, wrap)
}

fn open(
    cx: &mut App,
    bounds: Option<Bounds<Pixels>>,
    layout: Option<LayoutStore>,
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView + 'static,
) -> Result<WindowHandle<Root>> {
    open_with(cx, bounds, layout, |_, _, _| {}, wrap)
}

fn open_with(
    cx: &mut App,
    bounds: Option<Bounds<Pixels>>,
    layout: Option<LayoutStore>,
    mount: impl FnOnce(&Entity<MainView>, &mut Window, &mut App) + 'static,
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView + 'static,
) -> Result<WindowHandle<Root>> {
    let mut options = main_window_options(cx);
    if let Some(bounds) = bounds {
        options.window_bounds = Some(WindowBounds::Windowed(bounds));
    }
    cx.open_window(options, move |window, cx| {
        crate::session::windows::install_close_guard(window, cx);
        build_root_mounted(window, cx, layout, mount, wrap)
    })
    .context("opening the main window")
}

/// Builds the window root: [`MainView`] (wrapped by `wrap`) inside the `Root`. Also what the
/// headless screenshot renders, so the picture is of the real window content.
pub fn build_root(
    window: &mut Window,
    cx: &mut App,
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView,
) -> Entity<Root> {
    build_root_with_layout(window, cx, None, wrap)
}

/// [`build_root`] whose [`MainView`] restores (and then saves) the layout in `layout`, when given
/// ([`MainView::restoring`]).
pub fn build_root_with_layout(
    window: &mut Window,
    cx: &mut App,
    layout: Option<LayoutStore>,
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView,
) -> Entity<Root> {
    build_root_mounted(window, cx, layout, |_, _, _| {}, wrap)
}

/// [`build_root_with_layout`] with `mount` run on the [`MainView`] before `wrap` and the `Root`
/// (see [`open_main_window_mounted`]). Also what the headless screenshot of the app renders.
pub fn build_root_mounted(
    window: &mut Window,
    cx: &mut App,
    layout: Option<LayoutStore>,
    mount: impl FnOnce(&Entity<MainView>, &mut Window, &mut App),
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView,
) -> Entity<Root> {
    let main = cx.new(|cx| match layout {
        Some(store) => MainView::restoring(store, window, cx),
        None => MainView::new(window, cx),
    });
    mount(&main, window, cx);
    let content = wrap(main.into(), cx);
    new_root(content, window, cx)
}
