//! `cargo xtask perf`: scripted headless perf scenarios, a JSON report, and the regression gate
//! against `docs/perf/baseline.json` (E01-S14, ADR 0013, docs/PERFORMANCE.md).
//!
//! 1. Builds `oxikube` with `--features perf-scenarios` (profile `release-fast` by default).
//! 2. Per scenario: one warm-up process (discarded), then `--samples` fresh processes of
//!    `oxikube --perf-scenario <name> --perf-report <file>`. Each process is a cold start, and
//!    launch-to-first-frame is timed from here by waiting for its stdout marker.
//! 3. Aggregates: per metric, the median across samples of each statistic (p50/p95/p99/max), so a
//!    single noisy sample cannot fail the gate.
//! 4. Writes the report (`--out`, default `<target>/perf/report-<os>.json`) and prints a table.
//! 5. `--check`: compares with the baseline for this OS; fails when any p50/p95/p99 is more than
//!    `--tolerance` (20 %) AND more than the absolute noise floor slower: `--noise-floor-ms`
//!    (0.25 ms) for `*_ms` metrics, `--noise-floor-mib` (8 MiB) for `*_mib` memory metrics.
//!    Missing baselines are reported, not fatal; a baselined scenario that stops running is fatal.
//! 6. `--update-baseline`: writes this run's numbers into the baseline for this OS.
//! 7. Budgets (always, unless `--skip-budgets`): the absolute limits of ADR 0013 on the p95 across
//!    the run's cold launches (`budget::BUDGETS`: the first interactive frame ≤ 400 ms, failing
//!    beyond +20 %; settings + theme + keymap ≤ 30 ms, E05-S13; the table's first rows < 1 s and
//!    its frame under churn ≥ 55 fps, E07-S09).
//!
//! Every scenario process runs with `KUBECONFIG` set to the reference fixture: 3 kubeconfigs with
//! 20 contexts (`fixture`), and with `RUST_LIB_BACKTRACE=0` ([`sample_command`]).
//!
//! `--from-report <file>` skips 1-4 and applies `--check` / `--update-baseline` to a saved report
//! (a nightly `perf-report-<OS>` artifact): that is how the CI-runner baselines are seeded.

mod baseline;
mod budget;
mod fixture;
mod print;
mod report;

use anyhow::{Context, Result, bail};
use baseline::{Baseline, NoiseFloors, compare_with_tails};
use report::{
    HEADLESS_NOTE, REPORT_SCHEMA, Report, SAMPLE_SCHEMA, Sample, SampleStats, ScenarioResult,
    Status, aggregate,
};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

/// Every scenario `oxikube --perf-scenario` knows, in report order.
pub const SCENARIOS: [&str; 5] = [
    "startup",
    "scroll-10k",
    "palette",
    "logs-stream",
    "editor-5mb",
];

/// The name `oxikube --perf-scenario` reports for `name`: `table-scroll-10k` (the story's name
/// for it, E07-S09) is `scroll-10k`.
fn canonical_scenario(name: &str) -> &str {
    match name {
        "table-scroll-10k" => "scroll-10k",
        other => other,
    }
}

/// Must match `FIRST_FRAME_MARKER` in `bins/oxikube/src/perf_scenario.rs`.
const FIRST_FRAME_MARKER: &str = "OXIKUBE_PERF_FIRST_FRAME";
/// Metric added from outside the process.
const LAUNCH_METRIC: &str = "launch_to_first_frame_ms";

#[derive(clap::Args, Debug, Clone)]
pub struct Args {
    /// Scenario to run: startup, scroll-10k (alias table-scroll-10k), palette, logs-stream,
    /// editor-5mb.
    #[arg(required_unless_present_any = ["all", "from_report"], conflicts_with_all = ["all", "from_report"])]
    pub scenario: Option<String>,
    /// Run every scenario.
    #[arg(long, conflicts_with = "from_report")]
    pub all: bool,
    /// Do not run anything: load this report (e.g. a nightly `perf-report-<OS>` artifact) and
    /// apply `--check` / `--update-baseline` to it. This is how CI-runner baselines are seeded.
    #[arg(long)]
    pub from_report: Option<PathBuf>,
    /// Measured samples (fresh processes) per scenario, after one warm-up run.
    #[arg(long, default_value_t = 5)]
    pub samples: usize,
    /// Compare with the baseline and fail on a regression.
    #[arg(long)]
    pub check: bool,
    /// Write this run's numbers into the baseline for this OS. Cannot be combined with `--check`
    /// (the check would compare the run with the numbers just written and always pass).
    #[arg(long, conflicts_with = "check")]
    pub update_baseline: bool,
    /// Baseline file.
    #[arg(long, default_value = "docs/perf/baseline.json")]
    pub baseline: PathBuf,
    /// Report file (default `<target>/perf/report-<os>.json`).
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Allowed slowdown before `--check` fails (0.20 = +20 %).
    #[arg(long, default_value_t = 0.20)]
    pub tolerance: f64,
    /// Allowed slowdown of the p95 and p99 before `--check` fails, when it should differ from
    /// `--tolerance` (default: the same). Hosted CI runners move the tails first.
    #[arg(long)]
    pub tail_tolerance: Option<f64>,
    /// Absolute slowdown (ms) a `*_ms` metric must also exceed to fail; absorbs jitter on sub-ms
    /// metrics.
    #[arg(long, default_value_t = 0.25)]
    pub noise_floor_ms: f64,
    /// Absolute growth (MiB) a `*_mib` memory metric must also exceed to fail; absorbs allocator
    /// and loader jitter between runs (a few MiB), which +20 % of a small number would not.
    #[arg(long, default_value_t = 8.0)]
    pub noise_floor_mib: f64,
    /// Cargo profile for the scenario binary.
    #[arg(long, default_value = "release-fast")]
    pub profile: String,
    /// Use this prebuilt `oxikube` (built with `--features perf-scenarios`) instead of building.
    #[arg(long)]
    pub bin: Option<PathBuf>,
    /// Text recorded as the baseline's `source` with `--update-baseline` (default: CI run or
    /// `local <os>/<arch>`).
    #[arg(long)]
    pub source: Option<String>,
    /// Report the absolute budgets (ADR 0013) without failing on them (debugging a slow build).
    #[arg(long)]
    pub skip_budgets: bool,
}

pub fn run(args: &Args) -> Result<()> {
    if args.samples == 0 {
        bail!("--samples must be at least 1");
    }
    let metadata = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .context("cargo metadata")?;
    let root = metadata.workspace_root.as_std_path().to_owned();
    let target = metadata.target_directory.as_std_path().to_owned();

    let report = match &args.from_report {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading report {}", path.display()))?;
            let report: Report = serde_json::from_str(&text)
                .with_context(|| format!("parsing report {}", path.display()))?;
            if report.schema != REPORT_SCHEMA {
                bail!("report schema {} is not {REPORT_SCHEMA}", report.schema);
            }
            print::print_report(&report);
            report
        }
        None => run_and_write(args, &root, &target)?,
    };

    let budgets_failed = print::check_budgets(&report, args.skip_budgets);

    let baseline_path = if args.baseline.is_absolute() {
        args.baseline.clone()
    } else {
        root.join(&args.baseline)
    };
    if args.update_baseline {
        let mut baseline = Baseline::load_or_default(&baseline_path)?;
        let source = args
            .source
            .clone()
            .unwrap_or_else(|| match &args.from_report {
                Some(path) => format!("report {}", path.display()),
                None => default_source(),
            });
        baseline.update_from(&report, &source, &today());
        baseline.save(&baseline_path)?;
        println!(
            "baseline updated for `{}`: {}",
            report.os,
            baseline_path.display()
        );
    }
    if args.check {
        let baseline = Baseline::load(&baseline_path)?;
        let floors = NoiseFloors {
            ms: args.noise_floor_ms,
            mib: args.noise_floor_mib,
        };
        let tail_tolerance = args.tail_tolerance.unwrap_or(args.tolerance);
        let comparison =
            compare_with_tails(&report, &baseline, args.tolerance, tail_tolerance, floors);
        println!(
            "\ncheck against {} (os `{}`, fail above +{:.0} % (p95/p99 +{:.0} %) and +{} ms / +{} MiB):",
            baseline_path.display(),
            report.os,
            args.tolerance * 100.0,
            tail_tolerance * 100.0,
            floors.ms,
            floors.mib
        );
        for row in &comparison.rows {
            println!("  {row}");
        }
        if comparison.failed() {
            bail!(
                "perf check failed: see FAIL rows above (headless numbers; same-runner baseline)"
            );
        }
        println!("perf check passed");
    }
    if budgets_failed {
        bail!("perf budget exceeded: see FAIL rows in the budget table (ADR 0013)");
    }
    Ok(())
}

/// Runs the requested scenarios and writes the report.
fn run_and_write(args: &Args, root: &Path, target: &Path) -> Result<Report> {
    let scenarios: Vec<&str> = match (&args.scenario, args.all) {
        (_, true) => SCENARIOS.to_vec(),
        (Some(s), false) => {
            let s = canonical_scenario(s);
            let Some(known) = SCENARIOS.iter().find(|k| **k == s) else {
                bail!("unknown scenario `{s}`; known: {}", SCENARIOS.join(", "));
            };
            vec![*known]
        }
        (None, false) => bail!("give a scenario, --all or --from-report"),
    };
    let bin = match &args.bin {
        Some(bin) => bin.clone(),
        None => build(root, target, &args.profile)?,
    };
    let samples_dir = target.join("perf").join("samples");
    std::fs::create_dir_all(&samples_dir)?;
    let kubeconfig = fixture::write_kubeconfigs(&target.join("perf").join("fixture"))?;

    let mut report = Report {
        schema: REPORT_SCHEMA,
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        profile: args.profile.clone(),
        samples_per_scenario: args.samples,
        note: HEADLESS_NOTE.into(),
        scenarios: BTreeMap::new(),
    };
    for scenario in &scenarios {
        let result = run_scenario(&bin, scenario, args.samples, &samples_dir, &kubeconfig)?;
        report.scenarios.insert((*scenario).to_owned(), result);
    }

    let out = args.out.clone().unwrap_or_else(|| {
        target
            .join("perf")
            .join(format!("report-{}.json", report.os))
    });
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, serde_json::to_string_pretty(&report)? + "\n")
        .with_context(|| format!("writing {}", out.display()))?;
    print::print_report(&report);
    println!("\nreport: {}", out.display());
    Ok(report)
}

/// Builds the scenario binary and returns its path.
fn build(root: &Path, target: &Path, profile: &str) -> Result<PathBuf> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    println!("building oxikube --features perf-scenarios --profile {profile} ...");
    let status = Command::new(cargo)
        .current_dir(root)
        .args([
            "build",
            "-p",
            "oxikube",
            "--features",
            "perf-scenarios",
            "--profile",
            profile,
        ])
        .status()
        .context("running cargo build")?;
    if !status.success() {
        bail!("cargo build failed ({status})");
    }
    let dir = if profile == "dev" { "debug" } else { profile };
    let bin = target
        .join(dir)
        .join(format!("oxikube{}", std::env::consts::EXE_SUFFIX));
    if !bin.exists() {
        bail!("built binary not found at {}", bin.display());
    }
    Ok(bin)
}

/// Warm-up plus `samples` measured runs of one scenario.
fn run_scenario(
    bin: &Path,
    scenario: &str,
    samples: usize,
    dir: &Path,
    kubeconfig: &OsStr,
) -> Result<ScenarioResult> {
    print!("{scenario}: warm-up");
    let warm = run_sample(
        bin,
        scenario,
        &dir.join(format!("{scenario}-warmup.json")),
        kubeconfig,
    )?;
    if warm.status == "unavailable" {
        println!(" -> not available");
        return Ok(ScenarioResult {
            status: Status::Unavailable,
            reason: warm.reason,
            enabled_by: warm.enabled_by,
            samples: 0,
            metrics: BTreeMap::new(),
            counters: warm.counters,
            launches: BTreeMap::new(),
        });
    }
    let mut measured = Vec::with_capacity(samples);
    for i in 0..samples {
        print!(" {}", i + 1);
        measured.push(run_sample(
            bin,
            scenario,
            &dir.join(format!("{scenario}-{i}.json")),
            kubeconfig,
        )?);
    }
    println!();
    Ok(aggregate(&measured))
}

/// One fresh `oxikube --perf-scenario` process.
fn run_sample(bin: &Path, scenario: &str, report: &Path, kubeconfig: &OsStr) -> Result<Sample> {
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    let _ = std::fs::remove_file(report);
    let spawned = Instant::now();
    let mut child = sample_command(bin, scenario, report, kubeconfig)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning {}", bin.display()))?;
    let mut launch_to_first_frame = None;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let line = line?;
            if launch_to_first_frame.is_none() && line.trim() == FIRST_FRAME_MARKER {
                launch_to_first_frame = Some(spawned.elapsed());
            }
        }
    }
    let status = child.wait()?;
    if !status.success() {
        bail!("`oxikube --perf-scenario {scenario}` exited with {status}");
    }
    let text = std::fs::read_to_string(report)
        .with_context(|| format!("reading sample {}", report.display()))?;
    let mut sample: Sample =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", report.display()))?;
    if sample.schema != SAMPLE_SCHEMA {
        bail!(
            "sample schema {} from oxikube, xtask understands {SAMPLE_SCHEMA}; update both together",
            sample.schema
        );
    }
    if let Some(elapsed) = launch_to_first_frame {
        let ms = (elapsed.as_secs_f64() * 1_000_000.0).round() / 1000.0;
        sample.metrics.insert(
            LAUNCH_METRIC.into(),
            SampleStats {
                count: 1,
                p50: ms,
                p95: ms,
                p99: ms,
                max: ms,
            },
        );
    }
    Ok(sample)
}

/// The command line and environment of one sample process.
///
/// `RUST_LIB_BACKTRACE=0`: `cargo xtask` runs with the repository's `RUST_BACKTRACE=1`
/// (`.cargo/config.toml`), which the process would inherit, and with it every `anyhow` error
/// captures a stack trace. A hot path that builds errors then measures stack walks: GPUI's font
/// fallback did, about 300 ms a frame on the Linux runner (#509). Error backtraces off, panic
/// backtraces (`RUST_BACKTRACE`) kept.
fn sample_command(bin: &Path, scenario: &str, report: &Path, kubeconfig: &OsStr) -> Command {
    let mut command = Command::new(bin);
    command
        .args(["--perf-scenario", scenario, "--perf-report"])
        .arg(report)
        .env("KUBECONFIG", kubeconfig)
        .env("RUST_LIB_BACKTRACE", "0");
    command
}

fn default_source() -> String {
    match (
        std::env::var("GITHUB_ACTIONS").ok(),
        std::env::var("RUNNER_OS").ok(),
        std::env::var("GITHUB_RUN_ID").ok(),
    ) {
        (Some(_), Some(runner), Some(run)) => format!("github-actions {runner} run {run}"),
        _ => format!("local {}/{}", std::env::consts::OS, std::env::consts::ARCH),
    }
}

fn today() -> String {
    jiff::Timestamp::now().strftime("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: Args,
    }

    #[test]
    fn samples_run_with_error_backtraces_off() {
        let command = sample_command(
            Path::new("oxikube"),
            "scroll-10k",
            Path::new("sample.json"),
            OsStr::new("fixture"),
        );
        let env: BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(
            env.get(OsStr::new("RUST_LIB_BACKTRACE")),
            Some(&Some(OsStr::new("0"))),
            "error backtraces distort hot paths that build errors (#509)"
        );
        assert_eq!(
            env.get(OsStr::new("KUBECONFIG")),
            Some(&Some(OsStr::new("fixture")))
        );
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "--perf-scenario",
                "scroll-10k",
                "--perf-report",
                "sample.json"
            ]
        );
    }

    #[test]
    fn table_scroll_10k_is_scroll_10k() {
        assert_eq!(canonical_scenario("table-scroll-10k"), "scroll-10k");
        assert_eq!(canonical_scenario("startup"), "startup");
        assert!(SCENARIOS.contains(&canonical_scenario("table-scroll-10k")));
    }

    #[test]
    fn update_baseline_and_check_are_mutually_exclusive() {
        let err = Cli::try_parse_from(["perf", "--all", "--update-baseline", "--check"])
            .err()
            .expect("combination must be rejected");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
        assert!(Cli::try_parse_from(["perf", "--all", "--check"]).is_ok());
        assert!(Cli::try_parse_from(["perf", "--all", "--update-baseline"]).is_ok());
    }
}
