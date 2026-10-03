//! Smoke tests for headless screenshot mode (`OXIKUBE_SCREENSHOT`).
//!
//! Runs the real binary as a subprocess: on macOS the platform text system must be created on the
//! process main thread, which the libtest worker threads are not. Needs a GPU device (Metal, or
//! Vulkan such as Mesa lavapipe on Linux), so it only builds with `--features screenshot`
//! (nightly CI and local runs) and never in the default `cargo test --workspace` gate.
//! The subprocess approach also keeps the test free of OS threads waking GPUI tasks.
#![cfg(feature = "screenshot")]

use oxikube_testkit::screenshot::{distinct_colors_at_least, load_png};
use std::process::Command;

/// Window logical size (1280x800) times the headless scale factor (2).
const EXPECTED_SIZE: (u32, u32) = (2560, 1600);

fn oxikube() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_oxikube"));
    cmd.env_remove("OXIKUBE_SCREENSHOT");
    cmd
}

#[test]
fn renders_placeholder_at_window_pixel_size() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("nested/placeholder.png");
    let status = oxikube()
        .env("OXIKUBE_SCREENSHOT", &out)
        .status()
        .expect("run oxikube");
    assert!(status.success(), "oxikube exited with {status}");

    let image = load_png(&out).expect("PNG written");
    assert_eq!(image.dimensions(), EXPECTED_SIZE, "window pixel size");
    assert!(!image.as_raw().is_empty());
    // Background plus text glyphs: more than two colours means something was drawn.
    assert!(distinct_colors_at_least(&image, 3), "frame looks blank");
    // Placeholder background is 0x1e2127.
    assert_eq!(image.get_pixel(2, 2).0, [0x1e, 0x21, 0x27, 0xff]);
}

#[test]
fn unwritable_path_exits_non_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    // A path *under* a regular file cannot be created.
    let status = oxikube()
        .env("OXIKUBE_SCREENSHOT", file.join("out.png"))
        .status()
        .expect("run oxikube");
    assert!(!status.success(), "expected failure, got {status}");
}
