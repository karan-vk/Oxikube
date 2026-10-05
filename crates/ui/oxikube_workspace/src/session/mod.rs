//! Window and session basics (E05-S12): several windows, UI zoom, reduce-motion and the quit
//! confirmation.
//!
//! - [`settings`]: the root-level `ui_scale`, `reduce_motion` and `confirm_quit` settings
//!   ([`SessionSettings`]).
//! - [`zoom`]: `view::ZoomIn`, `view::ZoomOut` and `view::ZoomReset`. A zoom is applied to the UI
//!   at once (`oxikube_ui::set_ui_scale`, which the `Root` turns into the window's rem size on the
//!   next frame) and then written to `settings.json`; editing `ui_scale` by hand zooms the same
//!   way through hot reload.
//! - [`motion`]: the effective reduce-motion flag: the `reduce_motion` setting (`system`, `on`,
//!   `off`) over the OS preference, written to GPUI's `App::reduce_motion` so animations (ours and
//!   the component library's) follow it. Views ask `oxikube_ui::motion::reduce_motion`.
//! - [`quit`]: the [`Quit`] action and the quit guard. Features register providers of running
//!   operations (exec sessions, port-forwards, applies); a quit while any is running opens a
//!   confirm dialog on the window instead of quitting. The dialog is an overlay on the `Root`, so
//!   nothing blocks the UI thread.
//! - [`windows`]: `window::New` opens another main window with its own `Workspace`, and the
//!   per-window close guard (closing the last window on Linux and Windows quits, so it asks like
//!   `Quit` does; on macOS the app stays alive).
//!
//! Each user action is a GPUI action named after its `oxikube_domain::command` id (`view::ZoomIn`,
//! `window::New`, `app::Quit`), so the keymap, the palette and the MCP tools share one name; the
//! default key bindings are in the per-OS keymap files of `oxikube_assets` and, until the binary
//! loads those (E05-S07/S09), in `window::menus::default_bindings`.
//!
//! Order in the binary: settings, [`oxikube_ui::init`], then `oxikube_workspace::init`, which calls
//! [`init`]. Opening a second window runs none of this again: the globals (settings, tokens, zoom,
//! quit guard) are shared by all windows, and only the views are per window.

pub mod motion;
pub mod quit;
pub mod settings;
pub mod windows;
pub mod zoom;

#[cfg(test)]
mod tests;

use gpui::App;

pub use motion::{os_reduce_motion, resolve_reduce_motion, set_os_reduce_motion};
pub use quit::{
    OperationProviderId, Quit, RunningOperation, register_operation_provider, request_quit,
    running_operations, unregister_operation_provider,
};
pub use settings::{ReduceMotionSetting, SessionSettings, SessionSettingsContent};
pub use windows::{New as NewWindow, main_windows, open_new_window};
pub use zoom::{ZoomIn, ZoomOut, ZoomReset};

/// Registers the session actions, applies the settings (zoom, reduce-motion) and keeps them in
/// step with `settings.json`. Call once, after the settings store and `oxikube_ui::init`; works
/// (with defaults) when the settings store is installed later, too.
pub fn init(cx: &mut App) {
    zoom::register(cx);
    windows::register(cx);
    settings::apply_and_observe(cx);
}
