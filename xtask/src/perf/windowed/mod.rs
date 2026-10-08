//! `cargo xtask perf --windowed <scenario>|--all`: the real-window scenarios of ADR 0016.
//!
//! 1. Builds `oxikube --features perf-window` (profile `release-fast` by default) and copies it to
//!    `<target>/perf/windowed/oxikube`, so a headless build cannot replace it mid-run.
//! 2. Per scenario, `--samples` (default 5) fresh processes of `oxikube --perf-scenario-window
//!    <name>`, one after the other: each opens its window, activates itself, drives the scenario
//!    and writes its summary (`<target>/perf/windowed/<scenario>-<i>.summary.json`) next to the
//!    `--perf` JSONL (`<target>/perf/windowed/jsonl/`). Keep the window in front: a run during
//!    which it was not the active window is reported as no measurement.
//! 3. The `terminal` scenario runs its shell in a busybox pod on `--exec-context` (default
//!    `kind-oxikube`) when that context answers, else on this machine (`--exec-context none`
//!    forces the local shell).
//! 4. Writes `<target>/perf/windowed-report-<os>.json` (`--out`): every run's summary and, per
//!    figure ADR 0016 judges, the median and the worst run; prints the table.
//!
//! The budgets are reported, not enforced, unless `--enforce` is given: today's numbers are the
//! baseline the fix stories start from (E01-P587).

mod pod;
mod summary;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use summary::{Spread, Summary, WINDOWED_SCHEMA, aggregate};

/// Every scenario `oxikube --perf-scenario-window` knows, in report order (`Scenario::ALL`).
pub const SCENARIOS: [&str; 11] = [
    "pods-table",
    "table-filter",
    "namespaces",
    "detail-drawer",
    "tabs-panes",
    "theme",
    "catalog",
    "sidebar",
    "logs",
    "terminal",
    "idle",
];

/// Version of the [`Report`] format.
const REPORT_SCHEMA: u32 = 1;
/// Longest one run may take before it is killed.
const RUN_DEADLINE: Duration = Duration::from_secs(600);

/// What a windowed run is asked to do.
pub struct Options<'a> {
    pub scenario: Option<&'a str>,
    pub all: bool,
    pub samples: usize,
    pub profile: &'a str,
    pub bin: Option<&'a Path>,
    pub out: Option<&'a Path>,
    pub exec_context: &'a str,
    pub enforce: bool,
}

/// One scenario's runs.
#[derive(Debug, Serialize, Deserialize)]
pub struct ScenarioRuns {
    /// Runs whose window stayed active throughout (the others are not measurements).
    pub valid_runs: usize,
    /// Valid runs within every budget.
    pub passed_runs: usize,
    /// Per figure, the median run and the worst run (over the valid runs, or every run when none
    /// was valid).
    pub figures: BTreeMap<String, Spread>,
    /// Every distinct budget failure of the runs.
    pub failures: Vec<String>,
    /// Every run's summary.
    pub runs: Vec<Summary>,
}

/// `<target>/perf/windowed-report-<os>.json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    pub os: String,
    pub arch: String,
    pub profile: String,
    pub runs_per_scenario: usize,
    pub scenarios: BTreeMap<String, ScenarioRuns>,
}

/// Runs the windowed scenarios. See the [module docs](self).
pub fn run(options: &Options<'_>, root: &Path, target: &Path) -> Result<()> {
    let scenarios: Vec<&str> = match (options.scenario, options.all) {
        (_, true) => SCENARIOS.to_vec(),
        (Some(name), false) => match SCENARIOS.iter().find(|s| **s == name) {
            Some(known) => vec![*known],
            None => bail!(
                "unknown windowed scenario `{name}`; known: {}",
                SCENARIOS.join(", ")
            ),
        },
        (None, false) => bail!("give a scenario or --all"),
    };
    let dir = target.join("perf").join("windowed");
    std::fs::create_dir_all(dir.join("jsonl"))?;
    let bin = match options.bin {
        Some(bin) => bin.to_owned(),
        None => build(root, target, options.profile, &dir)?,
    };
    let mut report = Report {
        schema: REPORT_SCHEMA,
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        profile: options.profile.to_owned(),
        runs_per_scenario: options.samples,
        scenarios: BTreeMap::new(),
    };
    for scenario in scenarios {
        let pod = if scenario == "terminal" && options.exec_context != "none" {
            let pod = pod::ShellPod::start(options.exec_context)?;
            if pod.is_none() {
                println!(
                    "terminal: `{}` does not answer; the shell runs on this machine",
                    options.exec_context
                );
            }
            pod
        } else {
            None
        };
        let exec = pod.as_ref().map(pod::ShellPod::exec_arg);
        let mut runs = Vec::with_capacity(options.samples);
        print!("{scenario}:");
        for i in 0..options.samples {
            print!(" {}", i + 1);
            let _ = std::io::stdout().flush();
            runs.push(run_one(&bin, scenario, i, &dir, exec.as_deref())?);
        }
        println!();
        drop(pod);
        report
            .scenarios
            .insert(scenario.to_owned(), scenario_runs(runs));
    }
    let out = options.out.map_or_else(
        || {
            target
                .join("perf")
                .join(format!("windowed-report-{}.json", report.os))
        },
        Path::to_owned,
    );
    std::fs::write(&out, serde_json::to_string_pretty(&report)? + "\n")
        .with_context(|| format!("writing {}", out.display()))?;
    print(&report);
    println!("\nreport: {}", out.display());
    let over: Vec<&String> = report
        .scenarios
        .iter()
        .filter(|(_, s)| s.valid_runs == 0 || s.passed_runs < s.valid_runs)
        .map(|(name, _)| name)
        .collect();
    if options.enforce && !over.is_empty() {
        bail!(
            "over an ADR 0016 budget (or not measured): {}",
            over.iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}

fn scenario_runs(runs: Vec<Summary>) -> ScenarioRuns {
    let valid: Vec<Summary> = runs.iter().filter(|r| r.valid).cloned().collect();
    let figures = aggregate(if valid.is_empty() { &runs } else { &valid });
    let mut failures: Vec<String> = Vec::new();
    for failure in runs.iter().flat_map(|r| r.failures.iter()) {
        let kind = failure
            .split_once(|c: char| c.is_ascii_digit())
            .map_or(failure.as_str(), |(head, _)| head);
        if !failures.iter().any(|f| f.starts_with(kind)) {
            failures.push(failure.clone());
        }
    }
    ScenarioRuns {
        valid_runs: valid.len(),
        passed_runs: valid.iter().filter(|r| r.failures.is_empty()).count(),
        figures,
        failures,
        runs,
    }
}

/// Builds the windowed binary and copies it under `dir`.
fn build(root: &Path, target: &Path, profile: &str, dir: &Path) -> Result<PathBuf> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    println!("building oxikube --features perf-window --profile {profile} ...");
    let status = Command::new(cargo)
        .current_dir(root)
        .args([
            "build",
            "-p",
            "oxikube",
            "--features",
            "perf-window",
            "--profile",
            profile,
        ])
        .status()
        .context("running cargo build")?;
    if !status.success() {
        bail!("cargo build failed ({status})");
    }
    let built = target
        .join(if profile == "dev" { "debug" } else { profile })
        .join(format!("oxikube{}", std::env::consts::EXE_SUFFIX));
    let copy = dir.join(format!("oxikube{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(&built, &copy).with_context(|| format!("copying {}", built.display()))?;
    Ok(copy)
}

/// One fresh `oxikube --perf-scenario-window` process; its summary.
fn run_one(
    bin: &Path,
    scenario: &str,
    i: usize,
    dir: &Path,
    exec: Option<&str>,
) -> Result<Summary> {
    let report = dir.join(format!("{scenario}-{i}.summary.json"));
    let log = dir.join(format!("{scenario}-{i}.log"));
    let _ = std::fs::remove_file(&report);
    let mut child = run_command(bin, scenario, &report, &dir.join("jsonl"), exec)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log)?)
        .spawn()
        .with_context(|| format!("spawning {}", bin.display()))?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > RUN_DEADLINE {
            // Our own child only, by PID.
            let _ = child.kill();
            bail!(
                "`{scenario}` run {} took longer than {RUN_DEADLINE:?} (log: {})",
                i + 1,
                log.display()
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    if !status.success() {
        bail!(
            "`oxikube --perf-scenario-window {scenario}` exited with {status} (log: {})",
            log.display()
        );
    }
    let text = std::fs::read_to_string(&report)
        .with_context(|| format!("reading {} (log: {})", report.display(), log.display()))?;
    let summary: Summary =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", report.display()))?;
    if summary.schema != WINDOWED_SCHEMA {
        bail!(
            "summary schema {} from oxikube, xtask understands {WINDOWED_SCHEMA}; update both together",
            summary.schema
        );
    }
    Ok(summary)
}

/// The command line and environment of one run: error backtraces off (they distort hot paths
/// that build errors, #509), `/bin/sh` as the terminal's shell.
fn run_command(
    bin: &Path,
    scenario: &str,
    report: &Path,
    jsonl: &Path,
    exec: Option<&str>,
) -> Command {
    let mut command = Command::new(bin);
    command
        .args(["--perf-scenario-window", scenario, "--perf-report"])
        .arg(report)
        .arg("--perf-dir")
        .arg(jsonl)
        .env("RUST_LIB_BACKTRACE", "0")
        .env("SHELL", "/bin/sh");
    if let Some(exec) = exec {
        command.args(["--perf-exec", exec]);
    }
    command
}

fn print(report: &Report) {
    println!(
        "\n{} {} ({}), {} runs per scenario, real window; median run / worst run (ADR 0016: \
         every frame <= 8.33 ms, 0 dropped, input <= 1 frame, <= 1 notify per view per frame)",
        report.os, report.arch, report.profile, report.runs_per_scenario
    );
    println!(
        "{:<14} {:>5} {:>15} {:>15} {:>15} {:>13} {:>15} {:>9} {:>15} {:>13}",
        "scenario",
        "valid",
        "frame max",
        "frame p99",
        "frame p95",
        "dropped",
        "input max",
        "notif/vw",
        "peak RSS MiB",
        "CPU %"
    );
    for (name, s) in &report.scenarios {
        let f = |key: &str| {
            s.figures.get(key).map_or_else(
                || "-".to_owned(),
                |v| format!("{:.2}/{:.2}", v.median, v.worst),
            )
        };
        let n = |key: &str| {
            s.figures.get(key).map_or_else(
                || "-".to_owned(),
                |v| format!("{:.0}/{:.0}", v.median, v.worst),
            )
        };
        let cpu = if s.figures.contains_key("idle_cpu_percent") {
            f("idle_cpu_percent")
        } else {
            f("cpu_percent")
        };
        println!(
            "{name:<14} {:>5} {:>15} {:>15} {:>15} {:>13} {:>15} {:>9} {:>15} {:>13}",
            format!("{}/{}", s.valid_runs, s.runs.len()),
            f("frame_max_ms"),
            f("frame_p99_ms"),
            f("frame_p95_ms"),
            n("dropped_frames"),
            f("input_latency_max_ms"),
            n("max_view_notifies_per_frame"),
            f("peak_rss_mib"),
            cpu
        );
        for failure in &s.failures {
            println!("{:<14} over: {failure}", "");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_use_their_own_report_and_jsonl_dir_with_backtraces_off() {
        let command = run_command(
            Path::new("oxikube"),
            "terminal",
            Path::new("t.json"),
            Path::new("jsonl"),
            Some("kind-oxikube/ns/tty"),
        );
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "--perf-scenario-window",
                "terminal",
                "--perf-report",
                "t.json",
                "--perf-dir",
                "jsonl",
                "--perf-exec",
                "kind-oxikube/ns/tty"
            ]
        );
        let env: BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(
            env.get(std::ffi::OsStr::new("RUST_LIB_BACKTRACE")),
            Some(&Some(std::ffi::OsStr::new("0")))
        );
    }

    #[test]
    fn invalid_runs_do_not_count_as_measurements() {
        let example: Summary = serde_json::from_str(include_str!(
            "../../../../docs/perf/windowed-summary.example.json"
        ))
        .unwrap();
        let mut inactive = example.clone();
        inactive.valid = false;
        let runs = scenario_runs(vec![example.clone(), inactive]);
        assert_eq!(runs.valid_runs, 1);
        assert_eq!(runs.runs.len(), 2);
        assert_eq!(runs.passed_runs, usize::from(example.failures.is_empty()));
    }
}
