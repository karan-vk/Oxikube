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
}

#[test]
fn scenarios_without_views_are_not_available_and_exit_0() {
    for scenario in ["scroll-10k", "palette", "logs-stream", "editor-5mb"] {
        let (out, sample) = run(scenario);
        assert!(out.status.success(), "{scenario}");
        let s = sample.expect("sample written");
        assert_eq!(s["status"], "not_available", "{scenario}");
        assert!(s["enabled_by"][0].as_str().unwrap().starts_with("E05-S11"));
    }
}

#[test]
fn unknown_scenario_exits_2() {
    let (out, sample) = run("nope");
    assert_eq!(out.status.code(), Some(2));
    assert!(sample.is_none());
}
