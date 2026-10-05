//! Smoke tests for headless screenshot mode (`OXIKUBE_SCREENSHOT`).
//!
//! Runs the real binary as a subprocess: on macOS the platform text system must be created on the
//! process main thread, which the libtest worker threads are not. Needs a GPU device (Metal, or
//! Vulkan such as Mesa lavapipe on Linux), so it only builds with `--features screenshot`
//! (nightly CI and local runs) and never in the default `cargo test --workspace` gate.
//! The subprocess approach also keeps the test free of OS threads waking GPUI tasks.
#![cfg(feature = "screenshot")]

use oxikube_testkit::screenshot::{
    Tolerance, UPDATE_GOLDENS_ENV, check_golden, distinct_colors_at_least, golden_path, load_png,
};
use std::path::Path;
use std::process::Command;

/// Window logical size (1280x800) times the headless scale factor (2).
const EXPECTED_SIZE: (u32, u32) = (2560, 1600);

fn oxikube() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_oxikube"));
    cmd.env_remove("OXIKUBE_SCREENSHOT");
    cmd
}

#[test]
fn renders_main_window_at_window_pixel_size() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("nested/main_window.png");
    let status = oxikube()
        .env("OXIKUBE_SCREENSHOT", &out)
        .status()
        .expect("run oxikube");
    assert!(status.success(), "oxikube exited with {status}");

    let image = load_png(&out).expect("PNG written");
    assert_eq!(image.dimensions(), EXPECTED_SIZE, "window pixel size");
    assert!(!image.as_raw().is_empty());
    // Background, title bar, border and the title glyphs: more than a flat fill.
    assert!(distinct_colors_at_least(&image, 5), "frame looks blank");
    // The empty body is the dark `background` token (0x1e2127).
    let (w, h) = image.dimensions();
    assert_eq!(
        image.get_pixel(w / 2, h - 10).0,
        [0x1e, 0x21, 0x27, 0xff],
        "body is not the background token"
    );
    // The title bar (34 logical px = 68 device px) is drawn in its own, lighter colour.
    let bar = image.get_pixel(w / 2, 20).0;
    assert_ne!(bar, image.get_pixel(w / 2, h - 10).0, "no title bar drawn");

    // Golden for this OS, when one exists (`OXIKUBE_UPDATE_GOLDENS=1` regenerates it).
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let golden = golden_path(&goldens, "main_window");
    if golden.exists() || std::env::var_os(UPDATE_GOLDENS_ENV).is_some() {
        check_golden(&image, &golden, Tolerance::default()).expect("golden");
    }
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
