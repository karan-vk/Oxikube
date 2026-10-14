//! Smoke tests for `oxikube --perf-scenario` (feature `perf-scenarios`; needs a GPU device like the
//! screenshot tests, so it only runs where that feature is enabled: `cargo xtask perf` and local
//! runs). Runs the real binary as a subprocess because the macOS text system must be created on the
//! process main thread.
#![cfg(feature = "perf-scenarios")]

use serde_json::Value;
use std::process::Command;

fn oxikube() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_oxikube"));
    cmd.env_remove("OXIKUBE_SCREENSHOT");
    cmd
}

fn run(scenario: &str) -> (std::process::Output, Option<Value>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = dir.path().join("nested/sample.json");
    let out = oxikube()
        // `scroll-10k` in a debug build: a few frames are enough to check the sample.
        .env("OXIKUBE_PERF_SCROLL_FRAMES", SCROLL_FRAMES.to_string())
        // `logs-stream` likewise.
        .env("OXIKUBE_PERF_LOGS_FRAMES", LOGS_FRAMES.to_string())
        // `editor-5mb` on a smaller dump.
        .env("OXIKUBE_PERF_EDITOR_BYTES", EDITOR_BYTES.to_string())
        // `charts` likewise.
        .env("OXIKUBE_PERF_CHARTS_FRAMES", CHARTS_FRAMES.to_string())
        // `notifications-burst` likewise (the whole burst still arrives, in fewer frames).
        .env(
            "OXIKUBE_PERF_NOTIFICATIONS_FRAMES",
            NOTIFICATIONS_FRAMES.to_string(),
        )
        .args(["--perf-scenario", scenario, "--perf-report"])
        .arg(&report)
        .output()
        .expect("run oxikube");
    let sample = std::fs::read_to_string(&report)
        .ok()
        .map(|t| serde_json::from_str(&t).expect("sample is JSON"));
    (out, sample)
}

#[test]
fn startup_is_measured_and_prints_the_first_frame_marker() {
    let (out, sample) = run("startup");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("OXIKUBE_PERF_FIRST_FRAME"));
    let s = sample.expect("sample written");
    assert_eq!(s["schema"], 1);
    assert_eq!(s["status"], "ok");
    assert_eq!(s["counters"]["frames"], 120);
    assert_eq!(s["metrics"]["frame_ms"]["count"], 120);
    assert!(s["metrics"]["first_frame_ms"]["p50"].as_f64().unwrap() > 0.0);
    // E05-S13: the start-up breakdown and the budgets' inputs.
    for metric in [
        "config_load_ms",
        "state_db_open_ms",
        "init_logging_ms",
        "init_assets_ms",
        "init_settings_ms",
        "init_theme_ms",
        "init_keymap_ms",
        "init_ui_ms",
        "init_window_ms",
    ] {
        assert_eq!(s["metrics"][metric]["count"], 1, "{metric}");
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("network sockets before it: 0"),
        "no network before the first frame: {stderr}"
    );
}

/// Scripted frames of the `scroll-10k` smoke run.
const SCROLL_FRAMES: u64 = 20;

#[test]
fn scroll_10k_scrolls_the_table_under_churn_with_coalesced_notifies() {
    for name in ["scroll-10k", "table-scroll-10k"] {
        let (out, sample) = run(name);
        assert!(
            out.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let s = sample.expect("sample written");
        assert_eq!(s["status"], "ok");
        assert_eq!(
            s["scenario"], "scroll-10k",
            "the alias reports the scenario's name"
        );
        // Two frames per scripted frame: the coalesced feed notify's and the scroll's.
        assert_eq!(s["counters"]["frames"], 2 * SCROLL_FRAMES);
        assert_eq!(s["metrics"]["draw_ms"]["count"], SCROLL_FRAMES);
        assert_eq!(
            s["counters"]["feed_deltas"],
            10 * SCROLL_FRAMES,
            "ten events a batch"
        );
        assert_eq!(
            s["counters"]["notifies"], SCROLL_FRAMES,
            "one notify a batch"
        );
        assert_eq!(s["counters"]["max_notifies_per_frame"], 1);
        let first_rows = s["metrics"]["first_rows_ms"]["p50"].as_f64().unwrap();
        assert!(first_rows > 0.0, "{first_rows}");
    }
}

/// Scripted frames per mode of the `logs-stream` smoke run.
const LOGS_FRAMES: u64 = 10;

/// The `editor-5mb` dump in the smoke test: above the 1 MiB large-file threshold, so the
/// validation runs in chunks, small enough for a debug build.
const EDITOR_BYTES: u64 = 1_200_000;

#[test]
fn logs_stream_measures_ten_modes_with_coalesced_notifies() {
    let (out, sample) = run("logs-stream");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = sample.expect("sample written");
    assert_eq!(s["status"], "ok");
    assert_eq!(s["scenario"], "logs-stream");
    assert_eq!(s["metrics"]["draw_ms"]["count"], LOGS_FRAMES);
    for prefix in [
        "",
        "paused_",
        "wrap_",
        "wrap_paused_",
        "raw_",
        "json_filtered_",
        "search_",
        "filter_",
        "merged_",
        "merged_wrap_",
    ] {
        let frames = s["metrics"][format!("{prefix}frame_ms")]["count"]
            .as_u64()
            .unwrap_or_else(|| panic!("{prefix}frame_ms"));
        // A frame per scripted frame, and one more for each delta's coalesced notify.
        assert!(frames >= LOGS_FRAMES, "{prefix}frame_ms: {frames}");
    }
    assert!(
        s["counters"]["notifies"].as_u64().unwrap() > 0,
        "the deltas redrew"
    );
    assert_eq!(s["counters"]["max_notifies_per_frame"], 1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("lines/s"), "{stderr}");
}

#[test]
fn palette_opens_and_types_over_two_thousand_commands() {
    let (out, sample) = run("palette");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = sample.expect("sample written");
    assert_eq!(s["status"], "ok");
    assert_eq!(s["scenario"], "palette");
    // A frame per scripted frame, and the hook's frame for each redraw the matches cause.
    assert!(s["metrics"]["frame_ms"]["count"].as_u64().unwrap() >= 120);
    assert_eq!(s["metrics"]["draw_ms"]["count"], 120);
    let open = s["metrics"]["open_ms"]["p50"].as_f64().unwrap();
    assert!(open > 0.0, "{open}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("oxikube palette: opened"), "{stderr}");
}

#[test]
fn the_editor_opens_types_validates_and_scrolls_a_large_dump() {
    let (out, sample) = run("editor-5mb");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = sample.expect("sample written");
    assert_eq!(s["status"], "ok");
    assert_eq!(s["scenario"], "editor-5mb");
    // One typing frame and one scrolling frame per scripted frame.
    assert_eq!(s["metrics"]["draw_ms"]["count"], 120);
    assert_eq!(s["metrics"]["scroll_draw_ms"]["count"], 120);
    for metric in ["open_ms", "validation_ms", "frame_ms", "scroll_frame_ms"] {
        let p50 = s["metrics"][metric]["p50"].as_f64();
        assert!(p50.is_some_and(|v| v > 0.0), "{metric}: {p50:?}");
    }
    assert!(s["metrics"]["slow_frame_pct"]["p50"].as_f64().is_some());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("oxikube editor-5mb: opened"), "{stderr}");
}

/// Scripted frames per phase of the `charts` smoke run.
const CHARTS_FRAMES: u64 = 8;

#[test]
fn charts_repaint_from_cache_rebuild_once_per_tick_and_follow_a_hover() {
    let (out, sample) = run("charts");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = sample.expect("sample written");
    assert_eq!(s["status"], "ok");
    assert_eq!(s["scenario"], "charts");
    for prefix in ["", "tick_", "hover_"] {
        let frames = s["metrics"][format!("{prefix}draw_ms")]["count"].as_u64();
        assert_eq!(frames, Some(CHARTS_FRAMES), "{prefix}draw_ms");
        let p50 = s["metrics"][format!("{prefix}frame_ms")]["p50"].as_f64();
        assert!(p50.is_some_and(|v| v > 0.0), "{prefix}frame_ms: {p50:?}");
    }
}

/// Scripted frames per phase of the `notifications-burst` smoke run.
const NOTIFICATIONS_FRAMES: u64 = 8;

#[test]
fn a_notification_burst_reaches_the_centre_without_stacking_toasts() {
    let (out, sample) = run("notifications-burst");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The scenario fails its sample when more than the toast cap plus a summary ever show.
    assert!(out.status.success(), "{stderr}");
    let s = sample.expect("sample written");
    assert_eq!(s["status"], "ok");
    assert_eq!(s["scenario"], "notifications-burst");
    for prefix in ["", "idle_"] {
        let frames = s["metrics"][format!("{prefix}draw_ms")]["count"].as_u64();
        assert_eq!(frames, Some(NOTIFICATIONS_FRAMES), "{prefix}draw_ms");
        let p50 = s["metrics"][format!("{prefix}frame_ms")]["p50"].as_f64();
        assert!(p50.is_some_and(|v| v > 0.0), "{prefix}frame_ms: {p50:?}");
    }
    // The whole burst is listed, grouped: 50 reasons, three rows of their own and a group each.
    assert!(
        stderr.contains("oxikube notifications-burst: 1000 warnings over"),
        "{stderr}"
    );
    assert!(stderr.contains(", 200 rows listed,"), "{stderr}");
}

#[test]
fn unknown_scenario_exits_2() {
    let (out, sample) = run("nope");
    assert_eq!(out.status.code(), Some(2));
    assert!(sample.is_none());
}

/// Error backtraces on (`cargo` sets `RUST_BACKTRACE=1` through `.cargo/config.toml`) make every
/// error a hot path builds walk the stack, which made the Linux runner's frames ~300 ms (#509): a
/// sample says so on stderr; with `RUST_LIB_BACKTRACE=0` (what `cargo xtask perf` sets) it does not.
#[test]
fn a_sample_warns_when_error_backtraces_are_on() {
    const WARNING: &str = "error backtraces are on";
    let stderr_with = |value: &str| {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = oxikube()
            .env("RUST_LIB_BACKTRACE", value)
            .args(["--perf-scenario", "palette", "--perf-report"])
            .arg(dir.path().join("sample.json"))
            .output()
            .expect("run oxikube");
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    assert!(stderr_with("1").contains(WARNING));
    assert!(!stderr_with("0").contains(WARNING));
}
