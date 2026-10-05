//! `oxikube --perf-scenario <name>` (feature `perf-scenarios`): one headless sample of one scripted
//! scenario, written as a `ScenarioSample` JSON. `cargo xtask perf` runs this several times per
//! scenario in fresh processes and takes medians.
//!
//! Headless means GPUI's test platform with the host's real text system and headless renderer
//! (`oxikube_testkit::headless`): numbers are CPU, layout and paint preparation, no present and no
//! GPU time.
//!
//! Scenarios whose views do not exist yet report `not_available` (exit 0) with the stories that
//! enable them, so the harness, the nightly job and the baseline format are in place before them.

use anyhow::{Context as _, Result, bail};
use gpui::{AnyWindowHandle, AppContext as _, Pixels, Size, px, size};
use oxikube_runtime::perf::harness::{self, metric};
use oxikube_runtime::perf::{PerfRoot, Recorder, ScenarioSample, Summary, round_ms};
use oxikube_testkit::headless;
use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

/// Line printed to stdout (and flushed) the moment the first frame is drawn, so `cargo xtask perf`
/// can time launch-to-first-frame from outside the process.
pub const FIRST_FRAME_MARKER: &str = "OXIKUBE_PERF_FIRST_FRAME";

/// Logical window size for every scenario (the screenshot harness uses the same).
const WINDOW_SIZE: Size<Pixels> = size(px(1280.0), px(800.0));

/// Scripted redraws measured after the first frame.
const FRAMES: usize = 120;

/// The gpui `TestApp` harness every view-driving scenario needs.
const NEEDS_TEST_APP: &str = "E05-S11 #93";

/// Runs scenario `name`, writes its sample to `report` (or stdout). Exit 0 on success and for
/// `not_available`, 1 on failure, 2 for an unknown scenario.
pub fn run(name: &str, report: Option<&Path>, probe: bool, launched: Instant) -> ExitCode {
    let sample = match name {
        "startup" => startup(launched, probe),
        "scroll-10k" => Ok(ScenarioSample::not_available(
            name,
            "no pod table yet: scrolling 10k rows under 1 %/5 s churn needs the generic \
             ResourceTable fed from the load-pods fixture",
            &[NEEDS_TEST_APP, "E07-S01 #107", "E07-S03 #109"],
        )),
        "palette" => Ok(ScenarioSample::not_available(
            name,
            "no command palette yet: open <= 1 frame and filter 2 000 entries <= 5 ms",
            &[NEEDS_TEST_APP, "E11-S03 #158"],
        )),
        "logs-stream" => Ok(ScenarioSample::not_available(
            name,
            "no log viewer yet: streaming 5 000 lines/s",
            &[NEEDS_TEST_APP, "E08-S02 #120"],
        )),
        "editor-5mb" => Ok(ScenarioSample::not_available(
            name,
            "no manifest editor yet: 5 MB YAML open <= 500 ms and typing",
            &[NEEDS_TEST_APP, "E10-S04 #146", "E10-S11 #153"],
        )),
        other => {
            eprintln!(
                "oxikube: unknown perf scenario `{other}` (startup, scroll-10k, palette, \
                 logs-stream, editor-5mb)"
            );
            return ExitCode::from(2);
        }
    };
    match sample.and_then(|s| write_sample(&s, report)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("oxikube: perf scenario `{name}` failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn write_sample(sample: &ScenarioSample, report: Option<&Path>) -> Result<()> {
    let json = serde_json::to_string_pretty(sample)?;
    match report {
        Some(path) => {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, json + "\n").with_context(|| format!("writing {}", path.display()))
        }
        None => writeln!(std::io::stdout().lock(), "{json}").context("writing stdout"),
    }
}

/// Cold start to first frame drawn, then `FRAMES` idle redraws of the main view.
///
/// The real main window content, as `oxikube` builds it, in the headless context.
fn main_window(
    window: &mut gpui::Window,
    cx: &mut gpui::App,
    wrap: impl FnOnce(gpui::AnyView, &mut gpui::App) -> gpui::AnyView,
) -> gpui::Entity<oxikube_ui::root::Root> {
    oxikube_ui::init(cx);
    oxikube_workspace::window::build_root(window, cx, wrap)
}

/// `first_frame_ms` runs from the first line of `main` to the end of the window-opening update
/// (headless context with the platform text system and GPU renderer, window, first draw). Process
/// exec and dynamic loading before `main` are added from outside by `cargo xtask perf`
/// (`launch_to_first_frame_ms`).
fn startup(launched: Instant, probe: bool) -> Result<ScenarioSample> {
    let recorder = Arc::new(Recorder::new());
    let mut cx = headless::headless_context();
    let window: AnyWindowHandle = if probe {
        let recorder = recorder.clone();
        cx.open_window(WINDOW_SIZE, move |window, cx| {
            main_window(window, cx, move |content, cx| {
                cx.new(|_| PerfRoot::new(content, recorder)).into()
            })
        })?
        .into()
    } else {
        cx.open_window(WINDOW_SIZE, |window, cx| {
            main_window(window, cx, |content, _| content)
        })?
        .into()
    };
    // A test-mode context draws dirty windows when an update flushes its effects, so the window
    // has drawn its first frame by the time `open_window` returns.
    cx.run_until_parked();
    let first_frame = launched.elapsed();
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{FIRST_FRAME_MARKER}")?;
    stdout.flush()?;
    drop(stdout);
    if probe && recorder.frames_recorded() == 0 {
        bail!("no frame was drawn while opening the window");
    }

    let mut reader = recorder.reader();
    reader.drain(&recorder); // the first frame is reported as first_frame_ms, not frame_ms
    let run = harness::run_frames(
        &mut cx,
        window,
        FRAMES,
        &recorder,
        &mut reader,
        |cx| cx.run_until_parked(),
        |_, _, _| {},
    )?;
    Ok(run.into_sample(
        "startup",
        [(
            metric::FIRST_FRAME_MS.to_owned(),
            Summary::single(round_ms(first_frame.as_secs_f64() * 1000.0)),
        )],
    ))
}
