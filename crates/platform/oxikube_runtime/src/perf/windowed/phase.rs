//! The log of one phase of a windowed scenario ([`PhaseLog`]: its frames, refreshes and inputs,
//! and the counters at its edges) and its summary ([`summarise`]).

use std::time::{Duration, Instant};

use super::meter::RunInfo;
use super::summary::{PhaseKind, PhaseSummary};
use crate::perf::memory::{self, MemoryReading, bytes_to_mib};
use crate::perf::{FrameNotifies, Summary, cpu, round_ms};

/// Counters read from the process-wide recorder at a phase edge.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Edge {
    pub(super) at: Option<Instant>,
    pub(super) cpu: Option<Duration>,
    pub(super) feed_deltas: u64,
    pub(super) notifies: u64,
    pub(super) memory: Option<MemoryReading>,
}

impl Edge {
    pub(super) fn now(with_memory: bool) -> Self {
        let recorder = crate::perf::global();
        Self {
            at: Some(Instant::now()),
            cpu: cpu::process_time(),
            feed_deltas: recorder.map_or(0, |r| r.feed_deltas()),
            notifies: recorder.map_or(0, |r| r.notifies()),
            memory: if with_memory { memory::read() } else { None },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Frame {
    pub(super) start: Instant,
    /// Render start to the end of the content's paint (what the frame budget judges).
    pub(super) drawn: Duration,
    /// Render start to the end of the update that presented it.
    pub(super) presented: Duration,
    pub(super) notifies: FrameNotifies,
}

#[derive(Debug)]
pub(super) struct PhaseLog {
    pub(super) name: String,
    pub(super) kind: PhaseKind,
    pub(super) start: Edge,
    pub(super) end: Option<Edge>,
    pub(super) frames: Vec<Frame>,
    pub(super) refreshes: Vec<Instant>,
    pub(super) inactive_refreshes: u64,
    pub(super) activity_checks: u64,
    pub(super) inactive_checks: u64,
    pub(super) inputs: u64,
    pub(super) latencies_ns: Vec<u64>,
    pub(super) inputs_without_frame: u64,
}

impl PhaseLog {
    pub(super) fn new(name: &str, kind: PhaseKind) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            start: Edge::now(false),
            end: None,
            frames: Vec::new(),
            refreshes: Vec::new(),
            inactive_refreshes: 0,
            activity_checks: 0,
            inactive_checks: 0,
            inputs: 0,
            latencies_ns: Vec::new(),
            inputs_without_frame: 0,
        }
    }
}

pub(super) fn summarise(
    logs: &[PhaseLog],
    name: &str,
    kind: PhaseKind,
    info: &RunInfo,
) -> PhaseSummary {
    let refs: Vec<&PhaseLog> = logs.iter().collect();
    summarise_refs(&refs, name, kind, info)
}

/// One summary over `logs` (one phase, or every scripted phase together).
pub(super) fn summarise_refs(
    logs: &[&PhaseLog],
    name: &str,
    kind: PhaseKind,
    info: &RunInfo,
) -> PhaseSummary {
    let budget = Duration::from_secs_f64(info.budgets.frame_ms / 1000.0);
    let mut frames_ns: Vec<u64> = Vec::new();
    let mut presented_ns: Vec<u64> = Vec::new();
    let mut gaps_ns: Vec<u64> = Vec::new();
    let mut latencies_ns: Vec<u64> = Vec::new();
    let mut s = PhaseSummary {
        name: name.to_owned(),
        kind,
        duration_ms: 0.0,
        frames: None,
        presented_ms: None,
        over_budget_frames: 0,
        dropped_frames: 0,
        refreshes: 0,
        refresh_gap_ms: None,
        inactive_refreshes: 0,
        activity_checks: 0,
        inactive_checks: 0,
        inputs: 0,
        input_latency_ms: None,
        inputs_without_frame: 0,
        notifies: 0,
        max_notifies_per_frame: 0,
        max_view_notifies_per_frame: 0,
        feed_deltas: 0,
        cpu_percent: None,
        rss_mib: None,
        peak_rss_mib: None,
    };
    let mut wall = Duration::ZERO;
    let mut cpu_used: Option<Duration> = Some(Duration::ZERO);
    for log in logs {
        let end = log.end.unwrap_or_default();
        let phase_wall = match (log.start.at, end.at) {
            (Some(a), Some(b)) => b.saturating_duration_since(a),
            _ => Duration::ZERO,
        };
        wall += phase_wall;
        cpu_used = match (cpu_used, log.start.cpu, end.cpu) {
            (Some(acc), Some(a), Some(b)) => b.checked_sub(a).map(|d| acc + d),
            _ => None,
        };
        s.feed_deltas += end.feed_deltas.saturating_sub(log.start.feed_deltas);
        s.notifies += end.notifies.saturating_sub(log.start.notifies);
        if let Some(memory) = end.memory {
            s.rss_mib = Some(bytes_to_mib(memory.rss_bytes));
            let peak = bytes_to_mib(memory.peak_rss_bytes.unwrap_or(memory.rss_bytes));
            s.peak_rss_mib = Some(s.peak_rss_mib.map_or(peak, |p: f64| p.max(peak)));
        }
        for frame in &log.frames {
            frames_ns.push(nanos(frame.drawn));
            presented_ns.push(nanos(frame.presented));
            if frame.drawn > budget {
                s.over_budget_frames += 1;
            }
            s.max_notifies_per_frame = s.max_notifies_per_frame.max(frame.notifies.total);
            s.max_view_notifies_per_frame = s
                .max_view_notifies_per_frame
                .max(frame.notifies.max_per_view);
        }
        s.refreshes += log.refreshes.len() as u64;
        s.inactive_refreshes += log.inactive_refreshes;
        s.activity_checks += log.activity_checks;
        s.inactive_checks += log.inactive_checks;
        for pair in log.refreshes.windows(2) {
            let gap = pair[1].saturating_duration_since(pair[0]);
            gaps_ns.push(u64::try_from(gap.as_nanos()).unwrap_or(u64::MAX));
            if log.kind == PhaseKind::Driven {
                s.dropped_frames += missed_refreshes(gap, info.refresh);
            }
        }
        s.inputs += log.inputs;
        s.inputs_without_frame += log.inputs_without_frame;
        latencies_ns.extend_from_slice(&log.latencies_ns);
    }
    s.duration_ms = ms(wall);
    s.frames = Summary::from_nanos(&mut frames_ns);
    s.presented_ms = Summary::from_nanos(&mut presented_ns);
    s.refresh_gap_ms = Summary::from_nanos(&mut gaps_ns);
    s.input_latency_ms = Summary::from_nanos(&mut latencies_ns);
    s.cpu_percent = cpu::percent(Some(Duration::ZERO), cpu_used, wall);
    s
}

/// Refreshes skipped between two consecutive ones `gap` apart, on a display refreshing every
/// `refresh`: none up to one and a half intervals (timer and vsync jitter), else the whole
/// intervals beyond the first.
pub fn missed_refreshes(gap: Duration, refresh: Duration) -> u64 {
    if refresh.is_zero() || gap.as_secs_f64() <= refresh.as_secs_f64() * 1.5 {
        return 0;
    }
    let intervals = (gap.as_secs_f64() / refresh.as_secs_f64()).round() as u64;
    intervals.saturating_sub(1)
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

pub(super) fn ms(d: Duration) -> f64 {
    round_ms(d.as_secs_f64() * 1000.0)
}
