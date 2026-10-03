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
