//! Performance instrumentation behind `oxikube --perf` and `cargo xtask perf` (E01-S14, ADR 0013).
//!
//! # Shape
//!
//! - [`Recorder`]: the hot path. Recording a frame is a lock-free push into a single-producer
//!   ring buffer ([`FrameRing`]); feed deltas and `notify` calls are relaxed atomic adds. Nothing
//!   allocates, locks or does I/O on the UI thread.
//! - [`PerfSession`]: a background thread that drains the recorder every
//!   [`DEFAULT_FLUSH_INTERVAL`] and appends JSONL to `<dir>/oxikube-perf-*.jsonl`
//!   (the binary passes `<data dir>/perf`, honouring `OXIKUBE_DATA_DIR`).
//!   [`PerfSession::finish`] returns a [`SessionSummary`] (p50/p95/p99/max).
//! - [`PerfRoot`]: the frame hook. It wraps the window's root view; see "What a frame is" below.
//! - [`install`] / [`record_notify`] / [`record_feed_deltas`]: a process-wide recorder for code
//!   far from the window (feeds, `notify_coalesced`). When `--perf` is off nothing is installed
//!   and each call is one atomic load and a branch.
//! - [`memory`]: the process's resident memory (RSS and peak) per OS. The flush thread reads it
//!   once per tick; the scripted driver reads it between frames. Never inside a frame.
//! - [`FirstFrameProbe`]: runs a callback at the end of a window's first frame, the start-up
//!   marker (E05-S13); [`sockets`]: how many IPv4/IPv6 sockets the process holds, the "no network
//!   before the first frame" check.
//! - `harness` (feature `perf-harness`): drives a window frame by frame for scripted headless
//!   scenarios and builds a [`ScenarioSample`].
//!
//! # What a frame is
//!
//! gpui-pre 0.3.7 has no public frame-start/frame-end callback: the platform's `on_request_frame`
//! handler (private, in `gpui::window`) runs `window.draw(cx)` then `window.present()` inside one
//! `App::update`, and only its `profiler` feature (which instruments every executor task) records
//! draw times. So [`PerfRoot`] measures from the moment GPUI asks the root view to render (the
//! start of `draw_roots`, first thing in `Window::draw` after entity invalidation) to an
//! `App::defer` callback, which GPUI runs when it flushes effects at the end of that same update,
//! after `draw` and `present` returned. A frame therefore covers request-layout, prepaint and paint
//! of the whole tree, scene finish and (in the real app) the platform `present` call that encodes
//! and submits the GPU command buffer. It does not cover GPU execution or display latency, and it
//! counts only frames that actually re-render (GPUI skips clean windows).
//!
//! Headless runs (`cargo xtask perf`) use GPUI's test platform: there is no `present`, so the
//! numbers are CPU, layout and paint-preparation time only. Compare them against a same-runner
//! baseline, never against the absolute frame budget.

mod first_frame;
mod frame;
#[cfg(any(test, feature = "perf-harness"))]
pub mod harness;
pub mod memory;
mod recorder;
mod report;
mod ring;
mod session;
pub mod sockets;
mod stats;

pub use first_frame::FirstFrameProbe;
pub use frame::PerfRoot;
pub use recorder::{DEFAULT_FRAME_CAPACITY, Recorder, RecorderReader, Tick};
pub use report::{Counters, REPORT_SCHEMA, ScenarioSample, ScenarioStatus};
pub use ring::{FrameRing, RingReader};
pub use session::{
    DEFAULT_FLUSH_INTERVAL, FRAME_MEASURES, Finished, JSONL_SCHEMA, PerfSession, SessionSummary,
};
pub use stats::{Summary, nanos_to_ms, percentile_sorted, round_ms};

use std::sync::{Arc, OnceLock};

static GLOBAL: OnceLock<Arc<Recorder>> = OnceLock::new();

/// Installs the process-wide recorder. Returns `false` (and keeps the first one) if a recorder was
/// already installed. Call once, before the first window opens, only when `--perf` is on.
pub fn install(recorder: Arc<Recorder>) -> bool {
    GLOBAL.set(recorder).is_ok()
}

/// The process-wide recorder, if `--perf` installed one.
#[inline]
pub fn global() -> Option<&'static Arc<Recorder>> {
    GLOBAL.get()
}

/// Whether a process-wide recorder is installed (`--perf` is on).
#[inline]
pub fn enabled() -> bool {
    GLOBAL.get().is_some()
}

/// Counts one coalesced `cx.notify()` (call it where notifications are batched, e.g.
/// `notify_coalesced`). No-op unless `--perf` is on.
#[inline]
pub fn record_notify() {
    if let Some(recorder) = GLOBAL.get() {
        recorder.record_notify();
    }
}

/// Counts `n` feed deltas applied (watch events, log lines). No-op unless `--perf` is on.
#[inline]
pub fn record_feed_deltas(n: u64) {
    if let Some(recorder) = GLOBAL.get() {
        recorder.record_feed_deltas(n);
    }
}
