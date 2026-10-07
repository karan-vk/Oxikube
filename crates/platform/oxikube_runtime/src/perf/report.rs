//! One headless scenario run (`oxikube --perf-scenario <name> --perf-report <file>`).
//!
//! `cargo xtask perf` reads these files (it has its own mirror of the format so it does not build
//! GPUI); `docs/perf/scenario-sample.example.json` is the contract both sides test against.

use super::stats::Summary;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Version of the [`ScenarioSample`] format.
pub const REPORT_SCHEMA: u32 = 1;

/// Whether the scenario ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioStatus {
    /// Measured; `metrics` is filled.
    Ok,
    /// The view crate or required behavior is not built yet; `reason` and `enabled_by` explain why.
    Unavailable,
}

/// Counters observed during the scripted frames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    /// Frames recorded by the frame hook.
    pub frames: u64,
    /// Frames lost to ring overflow.
    pub dropped_frames: u64,
    /// Feed deltas applied.
    pub feed_deltas: u64,
    /// Coalesced notifies.
    pub notifies: u64,
    /// The most coalesced notifies between two consecutive frames (1 when the one streaming view
    /// is coalesced to frame cadence).
    #[serde(default)]
    pub max_notifies_per_frame: u64,
}

/// One sample (one process run) of one scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioSample {
    /// [`REPORT_SCHEMA`].
    pub schema: u32,
    /// Scenario name (`startup`, `scroll-10k`, ...).
    pub scenario: String,
    /// Whether it ran.
    pub status: ScenarioStatus,
    /// Why it did not run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Stories that make it runnable (`"E07-S03 #109"`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_by: Vec<String>,
    /// `std::env::consts::OS`.
    pub os: String,
    /// Always true today: GPUI test platform, no present, no GPU timing.
    pub headless: bool,
    /// Metric name (`first_frame_ms`, `frame_ms`, `draw_ms`) to its distribution in ms.
    #[serde(default)]
    pub metrics: BTreeMap<String, Summary>,
    /// Frame, feed and notify counters.
    #[serde(default)]
    pub counters: Counters,
}

impl ScenarioSample {
    /// A measured sample.
    pub fn ok(scenario: &str, metrics: BTreeMap<String, Summary>, counters: Counters) -> Self {
        Self {
            schema: REPORT_SCHEMA,
            scenario: scenario.into(),
            status: ScenarioStatus::Ok,
            reason: None,
            enabled_by: Vec::new(),
            os: std::env::consts::OS.into(),
            headless: true,
            metrics,
            counters,
        }
    }

    /// A scenario whose view crate or required behavior is not built yet.
    pub fn unavailable(scenario: &str, reason: &str, enabled_by: &[&str]) -> Self {
        Self {
            schema: REPORT_SCHEMA,
            scenario: scenario.into(),
            status: ScenarioStatus::Unavailable,
            reason: Some(reason.into()),
            enabled_by: enabled_by.iter().map(|s| (*s).to_owned()).collect(),
            os: std::env::consts::OS.into(),
            headless: true,
            metrics: BTreeMap::new(),
            counters: Counters::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed example parses and round-trips: the format `cargo xtask perf` reads.
    #[test]
    fn example_file_round_trips() {
        let text = include_str!("../../../../../docs/perf/scenario-sample.example.json");
        let sample: ScenarioSample = serde_json::from_str(text).unwrap();
        assert_eq!(sample.schema, REPORT_SCHEMA);
        assert_eq!(sample.status, ScenarioStatus::Ok);
        let back: serde_json::Value = serde_json::to_value(&sample).unwrap();
        let orig: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(back, orig);
    }

    #[test]
    fn unavailable_serialises_reason_and_no_metrics() {
        let s = ScenarioSample::unavailable(
            "palette",
            "crate not built yet",
            &["E11-S03 #158"],
        );
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["status"], "unavailable");
        assert_eq!(v["enabled_by"][0], "E11-S03 #158");
        assert!(v["metrics"].as_object().unwrap().is_empty());
    }
}
