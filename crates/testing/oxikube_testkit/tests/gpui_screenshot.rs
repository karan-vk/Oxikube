//! Screenshot example (E05-S11): drive a view with the same verbs as a `#[gpui::test]`
//! (`ScreenshotApp`), capture it, and compare it with a golden image.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread, which libtest worker threads are not. Needs a GPU device (Metal; Vulkan such as Mesa
//! lavapipe on Linux), so it only builds with `--features gpui-golden` and runs in the nightly
//! job: `cargo test -p oxikube_testkit --features gpui-golden --test gpui_screenshot`.
//!
//! Goldens live under `tests/goldens/<os>/` and are refreshed with `OXIKUBE_UPDATE_GOLDENS=1`
//! (see `docs/testing-gpui.md`). An OS without a golden only gets the structural checks.

mod support;

use std::process::ExitCode;

use anyhow::{Result, ensure};
use gpui::{AppContext as _, px, size};
use oxikube_testkit::{
    gpui_test::{GoldenCase, ScreenshotApp, run_golden_cases},
    screenshot::RgbaImage,
};
use support::{Counter, bindings};

const SIZE: (u32, u32) = (320, 120);

/// The counter after three key presses and a settled debounce.
fn counter_after_keystrokes() -> Result<RgbaImage> {
    let mut app = ScreenshotApp::new();
    app.bind_keys(bindings());
    let window = app.open_window(size(px(SIZE.0 as f32), px(SIZE.1 as f32)), |window, cx| {
        cx.new(|cx| Counter::new(window, cx))
    })?;
    app.simulate_keystrokes(window, "j j j")?;
    // Still pending: the label says so until the clock reaches the debounce.
    let pending = app.capture(window)?;
    app.advance_clock(support::SETTLE_AFTER);
    let settled = app.capture(window)?;
    ensure!(
        pending != settled,
        "advancing the clock past the debounce did not change the picture"
    );
    Ok(settled)
}

fn main() -> ExitCode {
    run_golden_cases(
        env!("CARGO_MANIFEST_DIR"),
        &[GoldenCase {
            name: "counter_settled",
            size: SIZE,
            render: counter_after_keystrokes,
        }],
    )
}
