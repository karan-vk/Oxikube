//! The per-process sample `oxikube --perf-scenario` writes, and the aggregated report.
//!
//! The sample format mirrors `oxikube_runtime::perf::ScenarioSample` (xtask does not depend on the
//! runtime crate, which would build GPUI). `docs/perf/scenario-sample.example.json` is the shared
//! contract; both crates test against it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Sample format version this xtask understands.
pub const SAMPLE_SCHEMA: u32 = 1;
/// Aggregated report format version.
pub const REPORT_SCHEMA: u32 = 1;

/// What headless numbers are; written into every report.
pub const HEADLESS_NOTE: &str = "Headless (GPUI test platform, real text system): CPU, layout and \
paint-preparation time only, no present and no GPU time. Compare against a same-runner baseline, \
not against the absolute frame budget. Memory (`*_mib`) is the headless process's resident set, in \
MiB: no swap chain, GPU surfaces or windowing-system state, so lower than the windowed app's RSS; \
compare it against the same-runner baseline too, never against the memory budget.";

/// Unit of a metric, from its name suffix: `*_mib` is MiB of resident memory, everything else
/// (`*_ms`) is milliseconds.
pub fn metric_unit(metric: &str) -> &'static str {
    if metric.ends_with("_mib") {
        "MiB"
    } else {
        "ms"
    }
}

/// Distribution of one metric in one sample, in the metric's unit (see [`metric_unit`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SampleStats {
    pub count: u64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

/// Counters from one sample.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub frames: u64,
    pub dropped_frames: u64,
    pub feed_deltas: u64,
    pub notifies: u64,
}

/// One `oxikube --perf-scenario` run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub schema: u32,
    pub scenario: String,
    /// `ok` or `not_available`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_by: Vec<String>,
    pub os: String,
    pub headless: bool,
    #[serde(default)]
    pub metrics: BTreeMap<String, SampleStats>,
    #[serde(default)]
    pub counters: Counters,
}

/// p50/p95/p99 (+max) of one metric after taking the median across samples, in the metric's unit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

impl Percentiles {
    pub fn stat(&self, name: &str) -> Option<f64> {
        match name {
            "p50" => Some(self.p50),
            "p95" => Some(self.p95),
            "p99" => Some(self.p99),
            _ => None,
        }
    }
}

/// Status of a scenario in the aggregated report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    NotAvailable,
}

/// One scenario in the aggregated report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_by: Vec<String>,
    /// Measured samples (warm-up excluded).
    pub samples: usize,
    #[serde(default)]
    pub metrics: BTreeMap<String, Percentiles>,
    /// Counters of the last measured sample (they are deterministic per scenario).
    #[serde(default)]
    pub counters: Counters,
}

/// `cargo xtask perf` output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    pub os: String,
    pub arch: String,
    pub profile: String,
    pub samples_per_scenario: usize,
    pub note: String,
    pub scenarios: BTreeMap<String, ScenarioResult>,
}

/// Median of `values` (mean of the middle two for an even count). `None` when empty.
pub fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    let m = if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    };
    Some((m * 1000.0).round() / 1000.0)
}

/// Aggregates measured samples of one scenario: per metric, the median of each statistic across
/// samples. Samples that do not carry a metric do not count towards it.
pub fn aggregate(samples: &[Sample]) -> ScenarioResult {
    let mut names: Vec<&String> = samples.iter().flat_map(|s| s.metrics.keys()).collect();
    names.sort();
    names.dedup();
    let metrics = names
        .into_iter()
        .filter_map(|name| {
            let of = |f: fn(&SampleStats) -> f64| {
                let mut v: Vec<f64> = samples
                    .iter()
                    .filter_map(|s| s.metrics.get(name).map(f))
                    .collect();
                median(&mut v)
            };
            Some((
                name.clone(),
                Percentiles {
                    p50: of(|s| s.p50)?,
                    p95: of(|s| s.p95)?,
                    p99: of(|s| s.p99)?,
                    max: of(|s| s.max),
                },
            ))
        })
        .collect();
    ScenarioResult {
        status: Status::Ok,
        reason: None,
        enabled_by: Vec::new(),
        samples: samples.len(),
        metrics,
        counters: samples.last().map(|s| s.counters).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(p50: f64, p99: f64) -> Sample {
        let mut metrics = BTreeMap::new();
        metrics.insert(
            "frame_ms".to_owned(),
            SampleStats {
                count: 10,
                p50,
                p95: p99,
                p99,
                max: p99,
            },
        );
        Sample {
            schema: SAMPLE_SCHEMA,
            scenario: "startup".into(),
            status: "ok".into(),
            reason: None,
            enabled_by: vec![],
            os: "linux".into(),
            headless: true,
            metrics,
            counters: Counters::default(),
        }
    }

    #[test]
    fn example_sample_parses() {
        let s: Sample = serde_json::from_str(include_str!(
            "../../../docs/perf/scenario-sample.example.json"
        ))
        .unwrap();
        assert_eq!(s.schema, SAMPLE_SCHEMA);
        assert_eq!(s.status, "ok");
        assert!(s.metrics.contains_key("first_frame_ms"));
        assert!(s.metrics.contains_key("frame_ms"));
    }

    #[test]
    fn units_follow_the_metric_name() {
        assert_eq!(metric_unit("rss_mib"), "MiB");
        assert_eq!(metric_unit("peak_rss_mib"), "MiB");
        assert_eq!(metric_unit("first_frame_ms"), "ms");
        assert_eq!(metric_unit("launch_to_first_frame_ms"), "ms");
    }

    #[test]
    fn example_sample_carries_memory() {
        let s: Sample = serde_json::from_str(include_str!(
            "../../../docs/perf/scenario-sample.example.json"
        ))
        .unwrap();
        assert_eq!(s.metrics["rss_mib"].count, 120);
        assert_eq!(s.metrics["peak_rss_mib"].count, 1);
    }

    #[test]
    fn median_odd_even_empty() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), Some(2.5));
        assert_eq!(median(&mut []), None);
    }

    #[test]
    fn aggregate_takes_the_median_per_statistic_and_resists_one_outlier() {
        let samples = [sample(1.0, 2.0), sample(1.2, 9.0), sample(1.1, 2.2)];
        let r = aggregate(&samples);
        let m = r.metrics["frame_ms"];
        assert_eq!((m.p50, m.p99), (1.1, 2.2));
        assert_eq!(r.samples, 3);
    }
}
