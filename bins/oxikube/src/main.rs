//! Oxikube binary. Wires adapters into the app and mounts the UI.
//!
//! Opens the themed main window (`oxikube_workspace::window`): the `Root` over a title bar and the
//! workspace, plus the application menu.
//!
//! Init order (Zed's `main.rs` pattern, E05-S09), documented stage by stage in [`oxikube::startup`]:
//! logging and the panic hook → assets → runtime → settings → theme → keymap → ui → state db
//! (opened in the background) → [`oxikube::app_state::AppState`] → workspace → feature crates → keymap
//! re-bind → open the window. Nothing waits on disk or network before the first frame; each stage
//! is timed in a `tracing` span, and the first interactive frame (budget: 400 ms from the first
//! line of `main`) is logged with the stage breakdown (E05-S13, printed to stderr under `--perf`).
//! The measured cost of each stage is in the [`oxikube::startup`] docs.
//!
//! Hidden tooling flags (`--print-settings-schema`, `--print-settings-crates`) make this binary the
//! generator behind `cargo xtask gen-settings-schema`: it links every crate that registers
//! settings, see [`settings_schema`].
//!
//! Flags (`oxikube --help`): `--perf` records frame times, feed throughput, notify counts and RSS
//! (docs/PERFORMANCE.md), `--perf-table` makes that run connect a context and scroll its pods
//! table, and `--perf-logs` makes it open a pod's log view; `--perf-scenario` runs one headless
//! perf sample (feature `perf-scenarios`, driven by `cargo xtask perf`), and
//! `--perf-scenario-window` one scripted scenario in the real window (feature `perf-window`,
//! `cargo xtask perf --windowed`, ADR 0016).

mod cli;
mod perf_mode;
#[cfg(feature = "perf-scenarios")]
mod perf_scenario;
#[cfg(feature = "screenshot")]
mod screenshot;
mod settings_schema;
mod window_scenario;

use gpui::{App, AppContext as _};
use oxikube::startup::{self, Stage, StartupEnv};
use oxikube_runtime::perf::{PerfRoot, Recorder};
use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    // First statement: the startup scenario measures from here.
    let launched = Instant::now();

    let args = match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Parsed::Run(args)) => args,
        Ok(cli::Parsed::Help) => {
            let _ = writeln!(std::io::stdout().lock(), "{}", cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(err) => {
            eprintln!("oxikube: {err}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };

    // Tooling flags: print and exit before logging, windows or any stage runs.
    if let Some(print) = args.print {
        return settings_schema::print(print);
    }

    if let Some(scenario) = &args.perf_scenario {
        #[cfg(feature = "perf-scenarios")]
        return perf_scenario::run(
            scenario,
            args.perf_report.as_deref(),
            !args.perf_no_probe,
            launched,
        );
        #[cfg(not(feature = "perf-scenarios"))]
        {
            let _ = (scenario, launched);
            eprintln!(
                "oxikube: --perf-scenario needs a build with `--features perf-scenarios` \
                 (`cargo xtask perf` builds one)"
            );
            return ExitCode::from(2);
        }
    }

    // Headless screenshot mode (dev/CI only; the feature is never in default or release builds).
    // Without the env var, or without the feature, behaviour is the normal window below.
    #[cfg(feature = "screenshot")]
    if let Some(path) = std::env::var_os(screenshot::ENV_VAR) {
        return screenshot::run(path.as_ref());
    }

    // Logging and the panic hook come first, so everything after this can log and a panic leaves
    // a crash file (stage 1 of the init order, `startup`).
    let boot = startup::boot(launched);

    let perf = if args.perf {
        match perf_mode::start(args.perf_dir.clone()) {
            Ok(recorder) => Some(recorder),
            Err(err) => {
                eprintln!("oxikube --perf: {err:#}");
                startup::shutdown();
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    let perf_duration = args.perf_duration;
    let drive = args
        .perf_table
        .clone()
        .map(|context| oxikube::perf_table::TableDrive {
            context,
            scroll: args.perf_scroll.unwrap_or(DEFAULT_SCROLL),
            also: args.perf_also.clone(),
        });
    let logs = args.perf_logs.as_deref().and_then(|value| {
        oxikube::perf_logs::LogsDrive::parse(
            value,
            args.perf_logs_wrap,
            args.perf_logs_paused,
            args.perf_logs_workload,
        )
    });
    let window = match args.perf_scenario_window.as_deref() {
        Some(name) => {
            match window_scenario::parse(name, args.perf_exec.as_deref(), args.perf_report.clone())
            {
                Ok(window) => Some(window),
                Err(code) => {
                    startup::shutdown();
                    return code;
                }
            }
        }
        None => None,
    };
    let drive = Drive {
        table: drive,
        logs,
        window,
    };

    run_app(boot, perf, perf_duration, drive);
    // Platforms where `run` returns after quitting (macOS exits from inside it; the quit hook
    // already finished the session there). Idempotent.
    perf_mode::finish();
    startup::shutdown();
    ExitCode::SUCCESS
}

/// Runs the GPUI application: registers the assets, runs the init order (`startup::init`), opens
/// the main window. With a `--perf` recorder the frame hook sits between the `Root` and the
/// content (the `Root` must remain the window's root view for overlays to work); without it
/// nothing is measured or paid for.
fn run_app(
    mut boot: startup::Boot,
    perf: Option<Arc<Recorder>>,
    perf_duration: Option<Duration>,
    drive: Drive,
) {
    let application = boot.report.time(Stage::Assets, || {
        gpui_platform::application().with_assets(oxikube_ui::Assets)
    });
    application.run(move |cx: &mut App| {
        let up = start(cx, boot, perf, perf_duration, drive);
        // Registered after every other quit observer (the window's persistence controller, the
        // perf recorder), so the log outlives their quit work; also on the failure paths, which
        // quit below. macOS exits from inside `run`, so `main` never gets to flush there.
        startup::flush_log_on_quit(cx);
        if up {
            cx.activate(true);
        } else {
            cx.quit();
        }
    });
}

/// The body of the `run` callback: the init order, then the main window. Returns whether the app
/// is up; on failure the error is already logged and printed and the caller quits.
fn start(
    cx: &mut App,
    boot: startup::Boot,
    perf: Option<Arc<Recorder>>,
    perf_duration: Option<Duration>,
    drive: Drive,
) -> bool {
    let env = match &drive.window {
        Some(window) => window_scenario::env(window, boot),
        None => StartupEnv::app(boot),
    };
    if let Err(err) = startup::init(cx, env) {
        tracing::error!(%err, "start-up failed");
        eprintln!("oxikube: {err}");
        return false;
    }
    // The window opens behind the startup placeholder (its saved layout is read in the
    // background) and its first frame ends start-up (`startup::first_frame`).
    let opened = startup::time_after_init(cx, Stage::Window, |cx| match perf {
        Some(recorder) => {
            perf_mode::attach(cx, perf_duration);
            startup::window::open_main_window(cx, move |content, cx| {
                cx.new(|_| PerfRoot::new(content, recorder)).into()
            })
        }
        None => startup::window::open_main_window(cx, |content, _| content),
    });
    let handle = match opened {
        Ok(handle) => handle,
        Err(err) => {
            tracing::error!(%err, "cannot open the main window");
            eprintln!("oxikube: {err:#}");
            return false;
        }
    };
    // After the window: the OS reduce-motion preference is read without delaying the first frame.
    oxikube::os_motion::follow(cx, oxikube::os_motion::system_probe());
    if let Some(table) = drive.table {
        oxikube::perf_table::start(table, handle.into(), cx);
    }
    if let Some(logs) = drive.logs {
        oxikube::perf_logs::start(logs, handle.into(), cx);
    }
    if let Some(window) = drive.window {
        window_scenario::start(window, handle.into(), cx);
    }
    true
}

/// What a `--perf` run drives in the window: the pods table (`--perf-table`), a log view
/// (`--perf-logs`), a windowed scenario (`--perf-scenario-window`), or nothing (a plain recorded
/// launch).
struct Drive {
    table: Option<oxikube::perf_table::TableDrive>,
    logs: Option<oxikube::perf_logs::LogsDrive>,
    window: Option<window_scenario::Window>,
}

/// Rows `--perf-table` scrolls per frame without `--perf-scroll`: a fast trackpad fling at
/// 120 Hz, as the `scroll-10k` scenario does.
const DEFAULT_SCROLL: usize = 3;
