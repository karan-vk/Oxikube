//! The windowed summaries `oxikube --perf-scenario-window` writes (a mirror of
//! `oxikube_runtime::perf::windowed::WindowedSummary`, so xtask does not build GPUI), and what the
//! report keeps of a scenario's runs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The summary format xtask reads (`WINDOWED_SCHEMA` in the runtime).
pub const WINDOWED_SCHEMA: u32 = 1;

/// p50/p95/p99/max of one distribution.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub count: u64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

/// One phase of a run (or the run's scripted phases together).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Phase {
    pub name: String,
    pub kind: String,
    pub duration_ms: f64,
    pub frames: Option<Stats>,
    #[serde(default)]
    pub presented_ms: Option<Stats>,
    pub over_budget_frames: u64,
    pub dropped_frames: u64,
    pub refreshes: u64,
    pub refresh_gap_ms: Option<Stats>,
    pub inactive_refreshes: u64,
    pub inputs: u64,
    pub input_latency_ms: Option<Stats>,
    pub inputs_without_frame: u64,
    pub notifies: u64,
    pub max_notifies_per_frame: u64,
    pub max_view_notifies_per_frame: u64,
    pub feed_deltas: u64,
    pub cpu_percent: Option<f64>,
    pub rss_mib: Option<f64>,
    pub peak_rss_mib: Option<f64>,
}

/// One frame over budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverBudget {
    pub phase: String,
    pub t_ms: f64,
    pub ms: f64,
}

/// One run's summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub schema: u32,
    pub scenario: String,
    pub os: String,
    pub arch: String,
    pub app_version: String,
    pub refresh_ms: f64,
    pub refresh_source: String,
    pub measures: String,
    pub budgets: serde_json::Value,
    pub scripted: Phase,
    pub phases: Vec<Phase>,
    pub over_budget: Vec<OverBudget>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub failures: Vec<String>,
    pub valid: bool,
}

impl Summary {
    /// The peak RSS of the run (the highest phase-end reading of the OS high-water mark).
    pub fn peak_rss_mib(&self) -> Option<f64> {
        self.phases
            .iter()
            .filter_map(|p| p.peak_rss_mib)
            .fold(None, |m, v| Some(m.map_or(v, |m: f64| m.max(v))))
    }

    /// CPU of the run's idle phases (the idle CPU figure), when it has any.
    pub fn idle_cpu_percent(&self) -> Option<f64> {
        self.phases
            .iter()
            .filter(|p| p.kind == "idle")
            .filter_map(|p| p.cpu_percent)
            .fold(None, |m, v| Some(m.map_or(v, |m: f64| m.max(v))))
    }
}

/// A figure across a scenario's runs: the median run and the worst run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Spread {
    pub median: f64,
    pub worst: f64,
}

impl Spread {
    fn of(mut values: Vec<f64>) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        values.sort_by(f64::total_cmp);
        let median = values[(values.len() - 1) / 2];
        let worst = values[values.len() - 1];
        Some(Self { median, worst })
    }
}

/// The figures ADR 0016 judges, across the runs of a scenario.
pub fn aggregate(runs: &[Summary]) -> BTreeMap<String, Spread> {
    let mut figures: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for run in runs {
        let s = &run.scripted;
        let mut add = |name: &'static str, value: Option<f64>| {
            if let Some(v) = value {
                figures.entry(name).or_default().push(v);
            }
        };
        add("frame_max_ms", s.frames.map(|f| f.max));
        add("frame_p99_ms", s.frames.map(|f| f.p99));
        add("frame_p95_ms", s.frames.map(|f| f.p95));
        add("frame_p50_ms", s.frames.map(|f| f.p50));
        add("presented_max_ms", s.presented_ms.map(|f| f.max));
        add("presented_p50_ms", s.presented_ms.map(|f| f.p50));
        add("over_budget_frames", Some(s.over_budget_frames as f64));
        add("dropped_frames", Some(s.dropped_frames as f64));
        add("input_latency_max_ms", s.input_latency_ms.map(|i| i.max));
        add("input_latency_p95_ms", s.input_latency_ms.map(|i| i.p95));
        add(
            "max_notifies_per_frame",
            Some(s.max_notifies_per_frame as f64),
        );
        add(
            "max_view_notifies_per_frame",
            Some(s.max_view_notifies_per_frame as f64),
        );
        add("cpu_percent", s.cpu_percent);
        add("idle_cpu_percent", run.idle_cpu_percent());
        add("peak_rss_mib", run.peak_rss_mib());
    }
    figures
        .into_iter()
        .filter_map(|(name, values)| Spread::of(values).map(|s| (name.to_owned(), s)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../../docs/perf/windowed-summary.example.json");

    #[test]
    fn the_example_summary_parses() {
        let summary: Summary = serde_json::from_str(EXAMPLE).unwrap();
        assert_eq!(summary.schema, WINDOWED_SCHEMA);
        assert_eq!(summary.scenario, "pods-table");
        assert!(summary.phases.iter().any(|p| p.kind == "setup"));
        assert!(summary.peak_rss_mib().is_some());
    }

    #[test]
    fn the_spread_is_the_median_and_the_worst_run() {
        let mut summary: Summary = serde_json::from_str(EXAMPLE).unwrap();
        summary.scripted.dropped_frames = 0;
        let mut slow = summary.clone();
        slow.scripted.frames.as_mut().unwrap().max = 40.0;
        slow.scripted.dropped_frames = 9;
        let mut fast = summary.clone();
        fast.scripted.frames.as_mut().unwrap().max = 2.0;
        let figures = aggregate(&[slow, summary.clone(), fast]);
        let max = figures["frame_max_ms"];
        assert_eq!(max.worst, 40.0);
        assert_eq!(max.median, summary.scripted.frames.unwrap().max);
        assert_eq!(figures["dropped_frames"].worst, 9.0);
        assert!(Spread::of(Vec::new()).is_none());
        assert_eq!(
            Spread::of(vec![3.0, 1.0]).unwrap(),
            Spread {
                median: 1.0,
                worst: 3.0
            }
        );
    }
}
