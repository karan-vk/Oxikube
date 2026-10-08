//! Stage 13: the main window, opened behind the startup placeholder.
//!
//! The window opens with the workspace's default layout while the saved layout is read through
//! the state db (`oxikube_workspace::window::open_main_window_mounted`): neither the SQLite open
//! nor the layout read is awaited before the first frame. The cluster UI is mounted in it before
//! that frame ([`crate::mount`]: the catalog home, hotbar and cluster tabs), and its content is
//! wrapped in the first-frame probe ([`super::first_frame::mark`]).

use anyhow::{Context as _, Result};
use gpui::{AnyView, App, AppContext as _, Entity, Window, WindowHandle};
use oxikube_runtime::perf::{FirstFrameProbe, PerfRoot};
use oxikube_ui::root::Root;
use oxikube_workspace::persistence::{LayoutStore, MAIN_WINDOW_ID};
use oxikube_workspace::window::MainView;

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

/// Opens the app's main window: restoring its layout, with the cluster UI mounted
/// ([`crate::mount::mount_main_window`]), probed for the first frame, with `wrap` applied to the
/// content first (the `--perf` frame hook).
pub fn open_main_window(
    cx: &mut App,
    wrap: impl FnOnce(AnyView, &mut App) -> AnyView + 'static,
) -> Result<WindowHandle<Root>> {
    let layout = main_layout_store(cx)?;
    oxikube_workspace::window::open_main_window_mounted(
        cx,
        Some(layout),
        crate::mount::mount_main_window,
        move |content, cx| {
            let content = wrap(content, cx);
            probe_first_frame(content, cx)
        },
    )
}

/// The [`MainView`] of an app window opened by [`open_main_window`] (under the `Root`, the
/// first-frame probe and, with `--perf`, the frame hook), `None` for any other window.
pub fn main_view(window: &Window, cx: &App) -> Option<Entity<MainView>> {
    let inner = probed_content(window, cx)?;
    let inner = match inner.clone().downcast::<PerfRoot>() {
        Ok(hook) => hook.read(cx).inner().clone(),
        Err(_) => inner,
    };
    inner.downcast::<MainView>().ok()
}

/// The `--perf` frame hook of an app window opened by [`open_main_window`] with one, `None`
/// without `--perf` or for any other window.
pub fn perf_root(window: &Window, cx: &App) -> Option<Entity<PerfRoot>> {
    probed_content(window, cx)?.downcast::<PerfRoot>().ok()
}

/// What the first-frame probe of an app window wraps.
fn probed_content(window: &Window, cx: &App) -> Option<AnyView> {
    let root = window.root::<Root>().flatten()?;
    let probe = root
        .read(cx)
        .view()
        .clone()
        .downcast::<FirstFrameProbe>()
        .ok()?;
    Some(probe.read(cx).inner().clone())
}
