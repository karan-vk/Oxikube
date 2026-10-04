//! The window root.
//!
//! gpui-component's `Root` hosts the dialog, sheet, notification, tooltip and menu overlay layers
//! and renders them once above the window content. Only `oxikube_ui` creates it: the window
//! (E05-S03) calls [`new_root`] with its workspace view as content, and feature views open
//! overlays through [`crate::dialog::OverlayExt`] without ever touching `Root`.

use gpui::{AnyView, App, AppContext as _, Context, Entity, Window};

pub use gpui_component::Root;

/// Builds the window root around `content`. Call from the closure given to `cx.open_window`:
///
/// ```ignore
/// cx.open_window(options, |window, cx| {
///     let view = cx.new(|cx| MyWorkspace::new(window, cx));
///     oxikube_ui::root::new_root(view, window, cx)
/// })
/// ```
pub fn new_root(content: impl Into<AnyView>, window: &mut Window, cx: &mut App) -> Entity<Root> {
    cx.new(|cx: &mut Context<Root>| Root::new(content, window, cx))
}
