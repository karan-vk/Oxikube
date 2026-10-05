//! Oxikube binary. Wires adapters into the app and mounts the UI.
//!
//! Opens the themed main window (`oxikube_workspace::window`): the `Root` over a title bar and an
//! empty workspace, plus the application menu. Init order will follow Zed's `main.rs` pattern
//! (E05-S09): logging → settings → keymap → theme → AppState → each crate's `init(cx)` →
//! workspace restore. Today: `oxikube_ui::init`, `oxikube_workspace::init`, open the window; nothing waits on
//! disk or network before the first frame.
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

    let perf = if args.perf {
        match perf_mode::start(args.perf_dir.clone()) {
            Ok(recorder) => Some(recorder),
            Err(err) => {
                eprintln!("oxikube --perf: {err:#}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    let perf_duration = args.perf_duration;

    run_app(perf, perf_duration);
    // Platforms where `run` returns after quitting (macOS exits from inside it; the quit hook
    // already finished the session there). Idempotent.
    perf_mode::finish();
    ExitCode::SUCCESS
}

/// Runs the GPUI application: registers the assets, initialises the UI stack, opens the main
/// window. With a `--perf` recorder the frame hook sits between the `Root` and the content (the
/// `Root` must remain the window's root view for overlays to work); without it nothing is
/// measured or paid for.
fn run_app(perf: Option<Arc<Recorder>>, perf_duration: Option<Duration>) {
    gpui_platform::application()
        .with_assets(oxikube_ui::Assets)
        .run(move |cx: &mut App| {
            oxikube_ui::init(cx);
            oxikube_workspace::init(cx);
            let opened = match perf {
                Some(recorder) => {
                    perf_mode::attach(cx, perf_duration);
                    oxikube_workspace::window::open_main_window_with(cx, move |content, cx| {
                        cx.new(|_| PerfRoot::new(content, recorder)).into()
                    })
                }
                None => oxikube_workspace::window::open_main_window(cx),
            };
            if let Err(err) = opened {
                eprintln!("oxikube: {err:#}");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}
