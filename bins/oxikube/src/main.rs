//! Oxikube binary. Wires adapters into the app and mounts the UI.
//!
//! Opens the themed main window (`oxikube_workspace::window`): the `Root` over a title bar and the
//! workspace, plus the application menu.
//!
//! Init order (Zed's `main.rs` pattern, E05-S09), documented stage by stage in [`oxikube::startup`]:
//! logging and the panic hook → runtime → assets → settings → theme → keymap → ui → state db
//! (opened in the background) → [`oxikube::app_state::AppState`] → workspace → feature crates → keymap
//! re-bind → open the window. Nothing waits on disk or network before the first frame; each stage
//! is timed in a `tracing` span.
//!
//! Flags (`oxikube --help`): `--perf` records frame times, feed throughput, notify counts and RSS
//! (docs/PERFORMANCE.md); `--perf-scenario` runs one headless perf sample (feature
//! `perf-scenarios`, driven by `cargo xtask perf`).

mod cli;
mod perf_mode;
#[cfg(feature = "perf-scenarios")]
mod perf_scenario;
#[cfg(feature = "screenshot")]
mod screenshot;

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
    let boot = startup::boot();

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

    run_app(boot, perf, perf_duration);
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
fn run_app(mut boot: startup::Boot, perf: Option<Arc<Recorder>>, perf_duration: Option<Duration>) {
    let application = boot.report.time(Stage::Assets, || {
        gpui_platform::application().with_assets(oxikube_ui::Assets)
    });
    application.run(move |cx: &mut App| {
        // Flush the log when the app quits (macOS exits from inside `run`, so `main` never gets
        // to drop the guard there).
        cx.on_app_quit(|_| {
            startup::shutdown();
            async {}
        })
        .detach();
        if let Err(err) = startup::init(cx, StartupEnv::app(boot)) {
            tracing::error!(%err, "start-up failed");
            eprintln!("oxikube: {err}");
            cx.quit();
            return;
        }
        let opened = startup::time_after_init(cx, Stage::Window, |cx| match perf {
            Some(recorder) => {
                perf_mode::attach(cx, perf_duration);
                oxikube_workspace::window::open_main_window_with(cx, move |content, cx| {
                    cx.new(|_| PerfRoot::new(content, recorder)).into()
                })
            }
            None => oxikube_workspace::window::open_main_window(cx),
        });
        if let Err(err) = opened {
            tracing::error!(%err, "cannot open the main window");
            eprintln!("oxikube: {err:#}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
