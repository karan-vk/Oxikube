//! Command-line smoke tests that need no window: `--help`, argument errors, and the perf flags'
//! error paths.

use std::process::Command;

fn oxikube() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_oxikube"));
    cmd.env_remove("OXIKUBE_SCREENSHOT");
    cmd
}

#[test]
fn help_lists_the_perf_flags() {
    let out = oxikube().arg("--help").output().expect("run oxikube");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for flag in [
        "--perf ",
        "--perf-duration",
        "--perf-dir",
        "--perf-scenario",
        "--perf-report",
    ] {
        assert!(text.contains(flag), "missing {flag} in:\n{text}");
    }
}

#[test]
fn unknown_argument_exits_2() {
    let out = oxikube().arg("--bogus").output().expect("run oxikube");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown argument `--bogus`"));
}

#[cfg(not(feature = "perf-scenarios"))]
#[test]
fn perf_scenario_needs_the_feature() {
    let out = oxikube()
        .args(["--perf-scenario", "startup"])
        .output()
        .expect("run oxikube");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--features perf-scenarios"));
}

/// The hidden generator flags behind `cargo xtask gen-settings-schema` (E05-S06b): the binary
/// prints a schema that contains every settings-owning crate's keys, and the checked-in schema is
/// exactly that output.
#[test]
fn print_settings_schema_covers_every_linked_crate_and_matches_the_checked_in_file() {
    let out = oxikube()
        .arg("--print-settings-schema")
        .output()
        .expect("run oxikube");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).expect("utf-8");
    let schema: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    let properties = schema["properties"].as_object().expect("properties");
    // One entry per settings-owning crate the app links: logging (`log`), theme (`theme`) and the
    // workspace session settings, which are root-level (`ui_scale`).
    for key in ["log", "theme", "ui_scale", "clusters"] {
        assert!(
            properties.contains_key(key),
            "`{key}` missing from the schema"
        );
    }
    assert_eq!(text, oxikube_assets::settings_schema());
}

#[test]
fn print_settings_crates_lists_each_registering_crate_once() {
    let out = oxikube()
        .arg("--print-settings-crates")
        .output()
        .expect("run oxikube");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    let crates: Vec<_> = text.lines().collect();
    for owner in ["oxikube_logging", "oxikube_theme", "oxikube_workspace"] {
        assert_eq!(crates.iter().filter(|c| **c == owner).count(), 1, "{text}");
    }
    let mut sorted = crates.clone();
    sorted.sort_unstable();
    assert_eq!(crates, sorted);
}

#[test]
fn the_print_flags_are_hidden_from_help() {
    let out = oxikube().arg("--help").output().expect("run oxikube");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("--print-settings"));
}
