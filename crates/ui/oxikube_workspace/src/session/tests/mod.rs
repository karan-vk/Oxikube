//! `#[gpui::test]`s of the session basics: zoom, reduce-motion, quit confirmation, windows, and
//! the command and key binding wiring.

mod commands;
mod motion;
mod quit;
mod quit_command;
mod windows;
mod zoom;

use std::{cell::Cell, path::Path, rc::Rc};

use gpui::{TestAppContext, VisualTestContext, WindowHandle};
use oxikube_ui::root::Root;
use tempfile::TempDir;

use crate::window::open_main_window;

/// Settings read from (and written to) a fresh config dir, the UI stack, and the workspace's
/// `init`, in the order the binary uses. Returns the dir, which must outlive the test.
pub(super) fn setup(cx: &mut TestAppContext) -> TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    cx.update(|cx| {
        oxikube_settings::init_with_dir(dir.path(), cx);
        oxikube_ui::init(cx);
        crate::init(cx);
    });
    dir
}

/// [`setup`] without a settings store.
pub(super) fn setup_without_settings(cx: &mut TestAppContext) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        crate::init(cx);
    });
}

/// Opens a main window and makes it the active one.
pub(super) fn open_window(cx: &mut TestAppContext) -> (WindowHandle<Root>, VisualTestContext) {
    let handle = cx
        .update(open_main_window)
        .expect("the main window opens in the test platform");
    let mut vcx = VisualTestContext::from_window(handle.into(), cx);
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    (handle, vcx)
}

/// Draws a frame, so the `Root` applies the rem size of the moment.
pub(super) fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| window.draw(cx).clear(cx));
}

/// The window's rem size after a frame.
pub(super) fn rem_size(vcx: &mut VisualTestContext) -> f32 {
    draw(vcx);
    vcx.update(|window, _| f32::from(window.rem_size()))
}

/// The text of `settings.json` in `dir`.
pub(super) fn settings_text(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("settings.json")).expect("settings.json exists")
}

/// Replaces quitting with a counter and returns it.
pub(super) fn count_quits(cx: &mut TestAppContext) -> Rc<Cell<usize>> {
    let quits = Rc::new(Cell::new(0));
    let counter = quits.clone();
    cx.update(|cx| {
        super::quit::set_quit_hook(cx, move |_| counter.set(counter.get() + 1));
    });
    quits
}
