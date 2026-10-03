//! Oxikube binary. Wires adapters into the app and mounts the UI.
//!
//! Until E05 lands this is a placeholder window proving the GPUI stack
//! (gpui-pre + gpui-component) resolves and renders on macOS and Linux.
//! Init order will follow Zed's `main.rs` pattern: logging → settings → keymap →
//! theme → AppState → each crate's `init(cx)` → workspace restore.
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

use gpui::{AnyWindowHandle, App, Context, Window, WindowOptions, div, prelude::*, rgb};
use oxikube_runtime::perf::PerfRoot;
use std::io::Write as _;
use std::process::ExitCode;
use std::time::Instant;

struct Placeholder;

impl Render for Placeholder {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(rgb(0x1e2127))
            .text_color(rgb(0xd7dae0))
            .text_xl()
            .child("Oxikube — workspace skeleton. See docs/ROADMAP.md.")
    }
}

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

    gpui_platform::application().run(move |cx: &mut App| {
        let opened = match perf {
            // `--perf`: the root is wrapped in the frame hook. Without it the window holds the
            // placeholder directly and nothing is measured or paid for.
            Some(recorder) => {
                perf_mode::attach(cx, perf_duration);
                cx.open_window(WindowOptions::default(), |_, cx| {
                    let inner = cx.new(|_| Placeholder);
                    cx.new(|_| PerfRoot::new(inner, recorder))
                })
                .map(AnyWindowHandle::from)
            }
            None => cx
                .open_window(WindowOptions::default(), |_, cx| cx.new(|_| Placeholder))
                .map(AnyWindowHandle::from),
        };
        opened.expect("open main window");
        cx.activate(true);
    });
    // Platforms where `run` returns after quitting (macOS exits from inside it; the quit hook
    // already finished the session there). Idempotent.
    perf_mode::finish();
    ExitCode::SUCCESS
}
