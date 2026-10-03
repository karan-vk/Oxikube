//! Headless screenshot mode (`--features screenshot`, dev and nightly CI only).
//!
//! `OXIKUBE_SCREENSHOT=out.png oxikube` renders the main view off-screen through
//! `Window::render_to_image` (via `oxikube_testkit::headless`), writes a PNG and exits: status 0
//! on success, 1 on any failure. No window is shown and nothing waits on wall-clock time; the
//! GPUI executor is driven deterministically until it is idle before the frame is captured.

use crate::Placeholder;
use gpui::{AppContext as _, Pixels, Size, px, size};
use oxikube_testkit::{headless, screenshot};
use std::{path::Path, process::ExitCode};

/// Environment variable naming the PNG to write.
pub const ENV_VAR: &str = "OXIKUBE_SCREENSHOT";

/// Logical size of the captured window. The PNG is this times
/// [`headless::HEADLESS_SCALE_FACTOR`].
pub const WINDOW_SIZE: Size<Pixels> = size(px(1280.0), px(800.0));

/// Renders the main view and writes it to `path`. Returns the process exit code.
pub fn run(path: &Path) -> ExitCode {
    match capture_to(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("oxikube: screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn capture_to(path: &Path) -> anyhow::Result<()> {
    let image = render()?;
    screenshot::save_png(&image, path)
}

/// Renders the main view into an image of `WINDOW_SIZE * HEADLESS_SCALE_FACTOR` pixels.
pub fn render() -> anyhow::Result<screenshot::RgbaImage> {
    headless::capture_view(WINDOW_SIZE, |_, cx| cx.new(|_| Placeholder))
}
