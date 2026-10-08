//! [`Meter`]: the phases of a windowed scenario and every frame, refresh and input in them.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::phase::{Edge, Frame, PhaseLog, ms, nanos, summarise};
use super::summary::{
    Budgets, MAX_LISTED_OVER_BUDGET, OverBudgetFrame, PhaseKind, PhaseSummary, WINDOWED_SCHEMA,
    WindowedSummary, failures,
};
use crate::perf::{FrameSample, FrameTap};

/// What the summary's frame figures cover.
pub const WINDOWED_MEASURES: &str = "frames: from the start of Window::draw to the end of the \
content's paint (layout, prepaint, paint), judged against the budget; presented_ms: to the end of \
the GPUI update that presented the frame (finishing the scene and present, which waits for a free \
drawable); dropped_frames: display refreshes missed while driven; input_latency_ms: from an \
input's dispatch to the end of the paint of the frame that shows it";

/// How the run is reported: what [`Meter::finish`] needs besides the measurements.
#[derive(Debug, Clone)]
pub struct RunInfo {
    /// The scenario's name.
    pub scenario: String,
    /// The binary's version.
    pub app_version: String,
    /// The display refresh interval dropped frames are counted against.
    pub refresh: Duration,
    /// Where `refresh` came from (`display` or `assumed`).
    pub refresh_source: String,
    /// The budgets to judge against.
    pub budgets: Budgets,
}

#[derive(Debug)]
struct State {
    origin: Option<Instant>,
    phases: Vec<PhaseLog>,
    pending_input: Option<Instant>,
}

/// The measurements of one windowed scenario, phase by phase. Cheap to clone (a shared handle);
/// UI thread only.
///
/// It starts in the `setup` phase. [`tap`](Self::tap) is the frame hook's callback (every frame
/// lands in the current phase); the pacer ([`super::drive`]) reports each display refresh and
/// [`input`](Self::input) each dispatched input; [`begin`](Self::begin) closes the current phase
/// (reading CPU time and memory at its edge, outside any frame) and opens the next.
#[derive(Debug, Clone)]
pub struct Meter {
    state: Rc<RefCell<State>>,
}

impl Default for Meter {
    fn default() -> Self {
        Self::new()
    }
}

impl Meter {
    /// A meter in its `setup` phase.
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(State {
                origin: None,
                phases: vec![PhaseLog::new("setup", PhaseKind::Setup)],
                pending_input: None,
            })),
        }
    }

    /// The callback for `PerfRoot::set_tap`: records every frame into the current phase, and
    /// closes the latency of the input waiting for it.
    pub fn tap(&self) -> FrameTap {
        let meter = self.clone();
        Rc::new(move |frame: FrameSample| meter.frame(frame))
    }

    /// Records one frame (what [`tap`](Self::tap) calls).
    pub fn frame(&self, frame: FrameSample) {
        let mut state = self.state.borrow_mut();
        state.origin.get_or_insert(frame.start);
        let pending = state.pending_input.take();
        let Some(phase) = state.phases.last_mut() else {
            return;
        };
        if let Some(input) = pending {
            // The frame that shows the input is drawn when its paint ends; `present` then hands it
            // to the display at the next refresh.
            let end = frame.start + frame.drawn.unwrap_or(frame.duration);
            let latency = end.saturating_duration_since(input);
            phase.latencies_ns.push(nanos(latency));
        }
        phase.frames.push(Frame {
            start: frame.start,
            drawn: frame.drawn.unwrap_or(frame.duration),
            presented: frame.duration,
            notifies: frame.notifies,
        });
    }

    /// Records a display refresh the pacer was called on; `active`: the window was the key window.
    pub fn refresh(&self, at: Instant, active: bool) {
        let mut state = self.state.borrow_mut();
        if let Some(phase) = state.phases.last_mut() {
            phase.refreshes.push(at);
            if !active {
                phase.inactive_refreshes += 1;
            }
        }
    }

    /// Records a check of the window's activity during an idle phase; `active`: it was the key
    /// window.
    pub fn activity(&self, active: bool) {
        let mut state = self.state.borrow_mut();
        if let Some(phase) = state.phases.last_mut() {
            phase.activity_checks += 1;
            if !active {
                phase.inactive_checks += 1;
            }
        }
    }

    /// Records that an input is dispatched now: its latency runs to the end of the next frame.
    /// An earlier input still without a frame is counted as never shown before this one.
    pub fn input(&self) {
        let now = Instant::now();
        let mut state = self.state.borrow_mut();
        let earlier = state.pending_input.replace(now);
        if let Some(phase) = state.phases.last_mut() {
            phase.inputs += 1;
            if earlier.is_some() {
                phase.inputs_without_frame += 1;
            }
        }
    }

    /// Closes the current phase and opens `name`.
    pub fn begin(&self, name: &str, kind: PhaseKind) {
        let mut state = self.state.borrow_mut();
        Self::close(&mut state);
        state.phases.push(PhaseLog::new(name, kind));
    }

    /// The name of the current phase.
    #[cfg(test)]
    pub fn phase(&self) -> String {
        self.state
            .borrow()
            .phases
            .last()
            .map(|p| p.name.clone())
            .unwrap_or_default()
    }

    /// Frames recorded in the current phase so far.
    #[cfg(test)]
    pub fn frames_in_phase(&self) -> usize {
        self.state
            .borrow()
            .phases
            .last()
            .map_or(0, |p| p.frames.len())
    }

    fn close(state: &mut State) {
        let pending = state.pending_input.take();
        if let Some(phase) = state.phases.last_mut()
            && phase.end.is_none()
        {
            if pending.is_some() {
                phase.inputs_without_frame += 1;
            }
            phase.end = Some(Edge::now(true));
        }
    }

    /// Closes the last phase and builds the summary.
    pub fn finish(&self, info: &RunInfo, notes: Vec<String>) -> WindowedSummary {
        let mut state = self.state.borrow_mut();
        Self::close(&mut state);
        let origin = state.origin;
        let phases: Vec<PhaseSummary> = state
            .phases
            .iter()
            .map(|p| summarise(&[p], &p.name, p.kind, info))
            .collect();
        let scripted_logs: Vec<&PhaseLog> = state
            .phases
            .iter()
            .filter(|p| p.kind != PhaseKind::Setup)
            .collect();
        let scripted = summarise(&scripted_logs, "scripted", PhaseKind::Driven, info);
        let budget = Duration::from_secs_f64(info.budgets.frame_ms / 1000.0);
        let over_budget = scripted_logs
            .iter()
            .flat_map(|p| {
                p.frames
                    .iter()
                    .filter(|f| f.drawn > budget)
                    .map(|f| OverBudgetFrame {
                        phase: p.name.clone(),
                        t_ms: origin.map_or(0.0, |o| ms(f.start.saturating_duration_since(o))),
                        ms: ms(f.drawn),
                    })
            })
            .take(MAX_LISTED_OVER_BUDGET)
            .collect();
        let mut notes = notes;
        let driven: u64 = scripted_logs
            .iter()
            .filter(|p| p.kind == PhaseKind::Driven)
            .map(|p| p.refreshes.len() as u64)
            .sum();
        if driven == 0 && scripted.activity_checks == 0 {
            notes.push("no display refresh was driven and no idle phase ran".to_owned());
        }
        if scripted.inactive_refreshes > 0 {
            notes.push(format!(
                "the window was not the active window for {} of {} driven refreshes (GPUI paces an \
                 inactive window at 30 fps): keep it in front for the whole run",
                scripted.inactive_refreshes, scripted.refreshes
            ));
        }
        if scripted.inactive_checks > 0 {
            notes.push(format!(
                "the window was not the active window at {} of {} checks of an idle phase: keep \
                 it in front for the whole run",
                scripted.inactive_checks, scripted.activity_checks
            ));
        }
        let measured = driven > 0 || scripted.activity_checks > 0;
        let valid = measured && scripted.inactive_refreshes == 0 && scripted.inactive_checks == 0;
        let mut summary = WindowedSummary {
            schema: WINDOWED_SCHEMA,
            scenario: info.scenario.clone(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            app_version: info.app_version.clone(),
            refresh_ms: ms(info.refresh),
            refresh_source: info.refresh_source.clone(),
            measures: WINDOWED_MEASURES.to_owned(),
            budgets: info.budgets,
            scripted,
            phases,
            over_budget,
            notes,
            failures: Vec::new(),
            valid,
        };
        summary.failures = failures(&summary);
        summary
    }
}
