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
//!
//! - `startup` ([`startup`]): the real init order, the main window behind the startup
//!   placeholder, the first interactive frame and the per-stage breakdown (E05-S13).
//! - `scroll-10k` ([`scroll_10k`], also `table-scroll-10k`): the resource table scrolling
//!   10 000 pods under feed churn, first rows after the feed is warm (E07-S09).
//! - `logs-stream` ([`logs_stream`]): the log view streaming 5 000 lines/s, wrap off/on and
//!   autoscroll on/paused (E08-S02), and with a search highlighting or filtering (E08-S03).

mod logs_stream;
mod scroll_10k;
mod startup;

use anyhow::{Context as _, Result};
use gpui::{Pixels, Size, px, size};
use oxikube_runtime::perf::ScenarioSample;
use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;
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
        "startup" => startup::run(launched, probe),
        scroll_10k::NAME | "table-scroll-10k" => scroll_10k::run(probe),
        "palette" => Ok(ScenarioSample::not_available(
            name,
            "no command palette yet: open <= 1 frame and filter 2 000 entries <= 5 ms",
            &[NEEDS_TEST_APP, "E11-S03 #158"],
        )),
        logs_stream::NAME => logs_stream::run(probe),
        "editor-5mb" => Ok(ScenarioSample::not_available(
            name,
            "no manifest editor yet: 5 MB YAML open <= 500 ms and typing",
            &[NEEDS_TEST_APP, "E10-S04 #146", "E10-S11 #153"],
        )),
        other => {
            eprintln!(
                "oxikube: unknown perf scenario `{other}` (startup, scroll-10k or \
                 table-scroll-10k, palette, logs-stream, editor-5mb)"
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
