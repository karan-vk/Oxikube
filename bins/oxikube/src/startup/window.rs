//! Stage 13: the main window, opened behind the startup placeholder.
//!
//! The window opens with the workspace's default layout while the saved layout is read through
//! the state db (`oxikube_workspace::window::open_main_window_restoring`): neither the SQLite open
//! nor the layout read is awaited before the first frame. Its content is wrapped in the
//! first-frame probe ([`super::first_frame::mark`]).

use anyhow::{Context as _, Result};
use gpui::{AnyView, App, AppContext as _, WindowHandle};
use oxikube_runtime::perf::FirstFrameProbe;
use oxikube_ui::root::Root;
use oxikube_workspace::persistence::{LayoutStore, MAIN_WINDOW_ID};

use crate::app_state::AppState;

/// The main window's saved layout, in the installed [`AppState`]'s state db.
///
/// # Errors
///
/// When [`AppState`] is not installed yet ([`super::init`] has not run).
pub fn main_layout_store(cx: &App) -> Result<LayoutStore> {
    let state = AppState::try_global(cx).context("start-up has not installed the AppState")?;
    LayoutStore::new(state.state().clone(), MAIN_WINDOW_ID).context("the main window's layout key")
}

/// Wraps the main window's content in the first-frame probe.
pub fn probe_first_frame(content: AnyView, cx: &mut App) -> AnyView {
    cx.new(|_| FirstFrameProbe::new(content, super::first_frame::mark))
        .into()
}

/// Opens the app's main window: restoring its layout, probed for the first frame, with `wrap`
/// applied to the content first (the `--perf` frame hook).
pub fn open_main_window(
    cx: &mut App,
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView + 'static,
) -> Result<WindowHandle<Root>> {
    let layout = main_layout_store(cx)?;
    oxikube_workspace::window::open_main_window_restoring(cx, layout, move |content, cx| {
        let content = wrap(content, cx);
        probe_first_frame(content, cx)
    })
}
