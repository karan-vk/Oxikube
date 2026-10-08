//! The per-scenario summary of a windowed run ([`WindowedSummary`]) and the ADR 0016 verdict.
//!
//! `cargo xtask perf --windowed` reads these files through its own mirror of the format (it does
//! not build GPUI); `WINDOWED_SCHEMA` versions it.

use serde::{Deserialize, Serialize};

use crate::perf::Summary;

/// Version of the [`WindowedSummary`] format.
pub const WINDOWED_SCHEMA: u32 = 1;

/// One refresh at 120 Hz, the reference display (ADR 0016): the budget of every frame, and of an
/// input's way to the screen.
pub const FRAME_BUDGET_MS: f64 = 1000.0 / 120.0;

/// How many over-budget frames the summary lists one by one (all of them are counted).
pub const MAX_LISTED_OVER_BUDGET: usize = 500;

/// The budgets a scenario is judged against (ADR 0016). The frame, dropped-frame, input and
/// notify budgets hold for every scenario; memory and idle CPU only where the scenario sets them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Budgets {
    /// Every scripted frame at most this long (ms): one refresh at 120 Hz.
    pub frame_ms: f64,
    /// Refreshes missed during the driven phases: none.
    pub dropped_frames: u64,
    /// From an input's dispatch to the end of the frame that shows it (ms): one refresh.
    pub input_latency_ms: f64,
    /// Coalesced notifies one view may receive between two frames.
    pub notifies_per_view_per_frame: u64,
    /// Peak resident memory (MiB), when the scenario has a memory budget (10 000 pods: 400 MB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_rss_mib: Option<f64>,
    /// CPU of the `idle` phases (% of one core), when the scenario has one (two clusters: 1 %).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_cpu_percent: Option<f64>,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            frame_ms: FRAME_BUDGET_MS,
            dropped_frames: 0,
            input_latency_ms: FRAME_BUDGET_MS,
            notifies_per_view_per_frame: 1,
            peak_rss_mib: None,
            idle_cpu_percent: None,
        }
    }
}

/// What a phase was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseKind {
    /// From the window's first frame to the first scripted phase: connecting, opening the views,
    /// the first list. Reported, not judged: it is a different budget (start-up, cluster open).
    Setup,
    /// Scripted: a step on every display refresh (scroll, type, switch, resize); frames, missed
    /// refreshes and input latency are judged.
    Driven,
    /// Nothing driven: what the app draws and spends on its own (feeds, timers); frames and CPU
    /// are judged.
    Idle,
}

/// One frame over [`Budgets::frame_ms`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverBudgetFrame {
    /// The phase it was drawn in.
    pub phase: String,
    /// When it started, ms since the scenario's window drew its first frame.
    pub t_ms: f64,
    /// How long it took, ms.
    pub ms: f64,
}

/// The numbers of one phase (or of all scripted phases together).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseSummary {
    /// Phase name (`setup`, `scroll`, `type-filter`, ...; `scripted` for the totals).
    pub name: String,
    /// What it was.
    pub kind: PhaseKind,
    /// Wall time, ms.
    pub duration_ms: f64,
    /// Frame times, ms (`None`: no frame drawn).
    pub frames: Option<Summary>,
    /// Frames over the frame budget.
    pub over_budget_frames: u64,
    /// Display refreshes the window missed while it was being driven (0 for setup and idle).
    pub dropped_frames: u64,
    /// Display refreshes the driver saw (one step each).
    pub refreshes: u64,
    /// Time between two refreshes the driver saw, ms (`None` with fewer than two).
    pub refresh_gap_ms: Option<Summary>,
    /// Refreshes during which the window was not the active (key) window. GPUI paces an
    /// inactive window at 30 fps, so a run with any is not a measurement of the budget.
    pub inactive_refreshes: u64,
    /// Inputs dispatched (keystrokes, scroll events, actions, commands).
    pub inputs: u64,
    /// From an input's dispatch to the end of the next frame, ms.
    pub input_latency_ms: Option<Summary>,
    /// Inputs after which no frame was drawn before the next input or the end of the phase.
    pub inputs_without_frame: u64,
    /// Coalesced notifies delivered.
    pub notifies: u64,
    /// The most coalesced notifies delivered between two frames, all views together.
    pub max_notifies_per_frame: u64,
    /// The most coalesced notifies one view received between two frames.
    pub max_view_notifies_per_frame: u64,
    /// Feed deltas (watch events, log lines) applied.
    pub feed_deltas: u64,
    /// Process CPU over the phase, % of one core.
    pub cpu_percent: Option<f64>,
    /// Resident memory at the end of the phase, MiB.
    pub rss_mib: Option<f64>,
    /// The process's peak resident memory at the end of the phase, MiB.
    pub peak_rss_mib: Option<f64>,
}

/// The per-scenario summary written next to the `--perf` JSONL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowedSummary {
    /// [`WINDOWED_SCHEMA`].
    pub schema: u32,
    /// The scenario (`pods-table`, `terminal`, ...).
    pub scenario: String,
    /// `std::env::consts::OS`.
    pub os: String,
    /// `std::env::consts::ARCH`.
    pub arch: String,
    /// The binary's version.
    pub app_version: String,
    /// The display's refresh interval the dropped frames are counted against, ms.
    pub refresh_ms: f64,
    /// Where `refresh_ms` came from (`display`: the OS; `assumed`: 120 Hz, no reader).
    pub refresh_source: String,
    /// What the frames include (the hook's definition).
    pub measures: String,
    /// The budgets the verdict applies.
    pub budgets: Budgets,
    /// Every scripted phase together: the numbers the budgets are about.
    pub scripted: PhaseSummary,
    /// Each phase, setup first.
    pub phases: Vec<PhaseSummary>,
    /// The scripted frames over the frame budget, in order (at most
    /// [`MAX_LISTED_OVER_BUDGET`]; `scripted.over_budget_frames` counts all).
    pub over_budget: Vec<OverBudgetFrame>,
    /// Facts about the run the reader needs (the window lost focus, a check of the scenario).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Which budgets the run broke; empty when it met them all.
    pub failures: Vec<String>,
    /// `false` when the run cannot speak for the budget (the window was not active throughout,
    /// or no refresh was driven): its numbers are reported, its failures are not a verdict.
    pub valid: bool,
}

impl WindowedSummary {
    /// Whether the run met every budget (and is a valid measurement).
    pub fn passed(&self) -> bool {
        self.valid && self.failures.is_empty()
    }

    /// One line per phase for stderr.
    pub fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .phases
            .iter()
            .chain(std::iter::once(&self.scripted))
            .map(|p| phase_line(&self.scenario, p))
            .collect();
        lines.push(match (self.valid, self.failures.is_empty()) {
            (false, _) => format!(
                "{}: NOT A MEASUREMENT ({})",
                self.scenario,
                self.notes.join("; ")
            ),
            (true, true) => format!("{}: within every ADR 0016 budget", self.scenario),
            (true, false) => format!(
                "{}: OVER BUDGET: {}",
                self.scenario,
                self.failures.join("; ")
            ),
        });
        lines
    }
}

fn phase_line(scenario: &str, p: &PhaseSummary) -> String {
    let frames = match &p.frames {
        Some(f) => format!(
            "{} frames: p50 {:.2}, p95 {:.2}, p99 {:.2}, max {:.2} ms; {} over budget",
            f.count, f.p50, f.p95, f.p99, f.max, p.over_budget_frames
        ),
        None => "no frame".to_owned(),
    };
    let input = p
        .input_latency_ms
        .map(|s| format!("; input p95 {:.2}, max {:.2} ms", s.p95, s.max))
        .unwrap_or_default();
    let cpu = p
        .cpu_percent
        .map(|c| format!("; cpu {c:.2} %"))
        .unwrap_or_default();
    let rss = match (p.rss_mib, p.peak_rss_mib) {
        (Some(r), Some(peak)) => format!("; rss {r:.1} MiB (peak {peak:.1})"),
        (Some(r), None) => format!("; rss {r:.1} MiB"),
        _ => String::new(),
    };
    format!(
        "{scenario} {} ({:?}, {:.1} s): {frames}; dropped {}{input}; notifies at most {} per \
         frame, {} per view{cpu}{rss}",
        p.name,
        p.kind,
        p.duration_ms / 1000.0,
        p.dropped_frames,
        p.max_notifies_per_frame,
        p.max_view_notifies_per_frame
    )
}

/// The budgets `summary` broke, in words (ADR 0016).
pub fn failures(summary: &WindowedSummary) -> Vec<String> {
    let b = &summary.budgets;
    let s = &summary.scripted;
    let mut out = Vec::new();
    if let Some(frames) = &s.frames
        && frames.max > b.frame_ms
    {
        out.push(format!(
            "{} of {} frames over {:.2} ms (max {:.2}, p99 {:.2}, p95 {:.2})",
            s.over_budget_frames, frames.count, b.frame_ms, frames.max, frames.p99, frames.p95
        ));
    }
    if s.dropped_frames > b.dropped_frames {
        out.push(format!("{} refreshes dropped", s.dropped_frames));
    }
    if let Some(input) = &s.input_latency_ms
        && input.max > b.input_latency_ms
    {
        out.push(format!(
            "input latency max {:.2} ms (p95 {:.2}) over one frame",
            input.max, input.p95
        ));
    }
    if s.inputs_without_frame > 0 {
        out.push(format!(
            "{} inputs never reached a frame before the next one",
            s.inputs_without_frame
        ));
    }
    if s.max_view_notifies_per_frame > b.notifies_per_view_per_frame {
        out.push(format!(
            "a view received {} coalesced notifies in one frame",
            s.max_view_notifies_per_frame
        ));
    }
    if let Some(limit) = b.peak_rss_mib {
        let peak = summary
            .phases
            .iter()
            .filter_map(|p| p.peak_rss_mib)
            .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
        if let Some(peak) = peak.filter(|p| *p >= limit) {
            out.push(format!("peak RSS {peak:.1} MiB, budget < {limit:.0}"));
        }
    }
    if let Some(limit) = b.idle_cpu_percent {
        for idle in summary.phases.iter().filter(|p| p.kind == PhaseKind::Idle) {
            if let Some(cpu) = idle.cpu_percent.filter(|c| *c >= limit) {
                out.push(format!("{}: CPU {cpu:.2} %, budget < {limit} %", idle.name));
            }
        }
    }
    out
}
