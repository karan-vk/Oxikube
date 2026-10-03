//! Headless GPUI rendering: draw a view into an [`RgbaImage`] without showing a window.
//!
//! Built on `gpui::HeadlessAppContext` (deterministic `TestDispatcher` scheduling, a real
//! platform text system, and the platform's headless GPU renderer: Metal on macOS, wgpu/Vulkan on
//! Linux) and `Window::render_to_image`. Both are gated behind `gpui/test-support` and
//! `gpui_platform/test-support`, which the `gpui-screenshot` feature turns on.
//!
//! Needs a GPU device at runtime. On Linux CI that means a Vulkan driver (Mesa lavapipe) and no
//! display server is required, since nothing is presented.
//!
//! macOS: the platform text system can only be created on the process main thread, so call these
//! from `main` (as `oxikube`'s screenshot mode does) or from a subprocess, not from libtest worker
//! threads.

use anyhow::{Context as _, Result};
use gpui::{App, Entity, HeadlessAppContext, Pixels, Render, Size, Window};
use image::RgbaImage;
use std::sync::Arc;

/// Device pixels per logical pixel in headless windows (fixed by GPUI's test window).
pub const HEADLESS_SCALE_FACTOR: u32 = 2;

/// Creates a [`HeadlessAppContext`] with the host's real text system (so text shapes and
/// measures like the real app) and the host's headless GPU renderer (so screenshots work).
pub fn headless_context() -> HeadlessAppContext {
    // `current_platform(true)` is the headless variant: it owns the platform text system but
    // opens no window and runs no event loop.
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(()), || {
        gpui_platform::current_headless_renderer()
    })
}

/// Opens a window of `size` (logical pixels) holding the view built by `build_root`, lets it
/// lay out and paint once, and returns what it rendered. The image is
/// `size * HEADLESS_SCALE_FACTOR` pixels.
///
/// Deterministic: no wall-clock waiting; pending tasks are run with `run_until_parked`.
pub fn capture_view<V: Render + 'static>(
    size: Size<Pixels>,
    build_root: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Result<RgbaImage> {
    let mut cx = headless_context();
    let window = cx
        .open_window(size, build_root)
        .context("opening headless window")?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .context("drawing headless window")?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
        .context("Window::render_to_image (needs a GPU device; on Linux a Vulkan driver)")
}
