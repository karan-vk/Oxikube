//! `docs/perf/baseline.json`: per-OS, per-scenario p50/p95/p99, and the regression check.

use super::report::{Percentiles, Report, Status};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// Baseline file format version.
pub const BASELINE_SCHEMA: u32 = 1;
/// Statistics the check compares.
pub const STATS: [&str; 3] = ["p50", "p95", "p99"];

/// The committed baseline.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    pub schema: u32,
    /// Free text: how the numbers are made and refreshed.
    #[serde(default)]
    pub note: String,
    /// `std::env::consts::OS` (`linux`, `macos`) to its numbers.
    #[serde(default)]
    pub os: BTreeMap<String, OsBaseline>,
}

/// Baseline numbers for one OS (one runner class).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OsBaseline {
    /// Where the numbers come from (`github-actions macos-latest run 123`, `local ...`).
    pub source: String,
    /// UTC date of the last `--update-baseline`.
    pub updated: String,
    pub profile: String,
    pub samples_per_scenario: usize,
    /// Scenario to metric to p50/p95/p99.
    pub scenarios: BTreeMap<String, BTreeMap<String, Percentiles>>,
}

impl Baseline {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading baseline {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Loads `path`, or an empty baseline if it does not exist yet.
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self {
                schema: BASELINE_SCHEMA,
                ..Self::default()
            })
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self)? + "\n";
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }

    /// Replaces this OS's numbers for every measured scenario in `report` (other scenarios and
    /// other OSes are kept). `max` is not stored: it is too noisy to gate on.
    pub fn update_from(&mut self, report: &Report, source: &str, updated: &str) {
        self.schema = BASELINE_SCHEMA;
        let entry = self.os.entry(report.os.clone()).or_default();
        entry.source = source.into();
        entry.updated = updated.into();
        entry.profile = report.profile.clone();
        entry.samples_per_scenario = report.samples_per_scenario;
        for (name, result) in &report.scenarios {
            if result.status != Status::Ok {
                continue;
            }
            let metrics = result
                .metrics
                .iter()
                .map(|(m, p)| (m.clone(), Percentiles { max: None, ..*p }))
                .collect();
            entry.scenarios.insert(name.clone(), metrics);
        }
    }
}

/// Outcome of one comparison row.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Within tolerance.
    Pass,
    /// Over the relative tolerance but by less than the absolute noise floor: not a regression.
    WithinNoiseFloor,
    /// Slower than baseline by more than the tolerance and the noise floor.
    Regressed,
    /// Nothing to compare against; reported, not fatal (seed it with `--update-baseline`).
    MissingBaseline(String),
    /// The baseline has numbers the run did not produce: the harness lost a scenario or metric.
    MissingInRun(String),
    /// Scenario not runnable yet (not in the baseline either).
    Skipped(String),
}

impl Outcome {
    pub fn is_failure(&self) -> bool {
        matches!(self, Outcome::Regressed | Outcome::MissingInRun(_))
    }
}

/// One line of the check table.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub scenario: String,
    pub metric: String,
    pub stat: String,
    pub baseline: Option<f64>,
    pub current: Option<f64>,
    pub outcome: Outcome,
}

impl Row {
    fn note(scenario: &str, metric: &str, outcome: Outcome) -> Self {
        Self {
            scenario: scenario.into(),
            metric: metric.into(),
            stat: String::new(),
            baseline: None,
            current: None,
            outcome,
        }
    }

    /// Relative change in percent, when both numbers exist.
    pub fn change_pct(&self) -> Option<f64> {
        match (self.baseline, self.current) {
            (Some(b), Some(c)) if b > 0.0 => Some((c / b - 1.0) * 100.0),
            _ => None,
        }
    }
}

impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match &self.outcome {
            Outcome::Pass | Outcome::WithinNoiseFloor => "PASS",
            Outcome::Regressed => "FAIL",
            Outcome::MissingBaseline(_) => "MISSING",
            Outcome::MissingInRun(_) => "FAIL",
            Outcome::Skipped(_) => "SKIPPED",
        };
        match (&self.outcome, self.baseline, self.current) {
            (Outcome::Pass | Outcome::WithinNoiseFloor | Outcome::Regressed, Some(b), Some(c)) => {
                write!(
                    f,
                    "{label:<8} {:<12} {:<26} {:<4} base {b:>10.3} ms  now {c:>10.3} ms  {:>+7.1} %",
                    self.scenario,
                    self.metric,
                    self.stat,
                    self.change_pct().unwrap_or(0.0)
                )?;
                if self.outcome == Outcome::WithinNoiseFloor {
                    write!(f, "  (under the noise floor)")?;
                }
                Ok(())
            }
            (Outcome::MissingBaseline(why), ..)
            | (Outcome::MissingInRun(why), ..)
            | (Outcome::Skipped(why), ..) => {
                write!(
                    f,
                    "{label:<8} {:<12} {:<26} {why}",
                    self.scenario, self.metric
                )
            }
            _ => write!(f, "{label:<8} {} {}", self.scenario, self.metric),
        }
    }
}

/// Result of [`compare`].
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub rows: Vec<Row>,
}

impl Comparison {
    pub fn failed(&self) -> bool {
        self.rows.iter().any(|r| r.outcome.is_failure())
    }
}

/// Whether `current` regressed against `base`: more than `tolerance` (0.20 = +20 %) slower AND
/// more than `noise_floor_ms` slower in absolute terms (sub-floor jitter on microsecond-scale
/// metrics is noise, not a regression).
pub fn regressed(base: f64, current: f64, tolerance: f64, noise_floor_ms: f64) -> bool {
    current > base * (1.0 + tolerance) && current - base > noise_floor_ms
}

/// Compares every scenario in `report` with the baseline for `report.os`.
///
/// - measured scenario/metric/stat: PASS or FAIL (see [`regressed`]);
/// - no baseline for this OS, scenario or metric: MISSING (not fatal; seed with
///   `--update-baseline`);
/// - baseline has a scenario or metric the run did not measure: FAIL;
/// - scenario not available and not in the baseline: SKIPPED.
///
/// Scenarios not in `report` (not requested on this run) are not compared.
pub fn compare(
    report: &Report,
    baseline: &Baseline,
    tolerance: f64,
    noise_floor_ms: f64,
) -> Comparison {
    let mut rows = Vec::new();
    let os = baseline.os.get(&report.os);
    for (scenario, result) in &report.scenarios {
        let base = os.and_then(|o| o.scenarios.get(scenario));
        match (result.status, base) {
            (Status::NotAvailable, None) => rows.push(Row::note(
                scenario,
                "-",
                Outcome::Skipped(format!(
                    "not available yet ({}); enabled by {}",
                    result.reason.as_deref().unwrap_or("no reason given"),
                    if result.enabled_by.is_empty() {
                        "-".to_owned()
                    } else {
                        result.enabled_by.join(", ")
                    }
                )),
            )),
            (Status::NotAvailable, Some(_)) => rows.push(Row::note(
                scenario,
                "-",
                Outcome::MissingInRun(format!(
                    "baseline has numbers for `{scenario}` on {} but this run reports it as not available",
                    report.os
                )),
            )),
            (Status::Ok, None) => rows.push(Row::note(
                scenario,
                "-",
                Outcome::MissingBaseline(match os {
                    None => format!(
                        "no baseline for os `{}` in docs/perf/baseline.json; seed it with \
                         `cargo xtask perf --all --update-baseline` on that runner",
                        report.os
                    ),
                    Some(_) => format!(
                        "scenario `{scenario}` has no baseline for os `{}`; seed it with \
                         `cargo xtask perf {scenario} --update-baseline`",
                        report.os
                    ),
                }),
            )),
            (Status::Ok, Some(base)) => {
                for (metric, b) in base {
                    let Some(cur) = result.metrics.get(metric) else {
                        rows.push(Row::note(
                            scenario,
                            metric,
                            Outcome::MissingInRun(format!(
                                "metric `{metric}` is in the baseline but this run did not measure it"
                            )),
                        ));
                        continue;
                    };
                    for stat in STATS {
                        let (bv, cv) = (b.stat(stat).unwrap_or(0.0), cur.stat(stat).unwrap_or(0.0));
                        rows.push(Row {
                            scenario: scenario.clone(),
                            metric: metric.clone(),
                            stat: stat.into(),
                            baseline: Some(bv),
                            current: Some(cv),
                            outcome: if regressed(bv, cv, tolerance, noise_floor_ms) {
                                Outcome::Regressed
                            } else if regressed(bv, cv, tolerance, 0.0) {
                                Outcome::WithinNoiseFloor
                            } else {
                                Outcome::Pass
                            },
                        });
                    }
                }
                for metric in result.metrics.keys().filter(|m| !base.contains_key(*m)) {
                    rows.push(Row::note(
                        scenario,
                        metric,
                        Outcome::MissingBaseline(format!(
                            "metric `{metric}` has no baseline yet; refresh with --update-baseline"
                        )),
                    ));
                }
            }
        }
    }
    Comparison { rows }
}

#[cfg(test)]
mod tests {
    use super::super::report::{Counters, ScenarioResult};
    use super::*;

    const TOL: f64 = 0.20;
    const FLOOR: f64 = 0.25;

    fn pct(v: f64) -> Percentiles {
        Percentiles {
            p50: v,
            p95: v,
            p99: v,
            max: None,
        }
    }

    fn report(scenarios: &[(&str, Option<f64>)]) -> Report {
        Report {
            schema: 1,
            os: "linux".into(),
            arch: "x86_64".into(),
            profile: "release-fast".into(),
            samples_per_scenario: 5,
            note: String::new(),
            scenarios: scenarios
                .iter()
                .map(|(name, value)| {
                    let result = match value {
                        Some(v) => ScenarioResult {
                            status: Status::Ok,
                            reason: None,
                            enabled_by: vec![],
                            samples: 5,
                            metrics: [("first_frame_ms".to_owned(), pct(*v))].into(),
                            counters: Counters::default(),
                        },
                        None => ScenarioResult {
                            status: Status::NotAvailable,
                            reason: Some("no view".into()),
                            enabled_by: vec!["E11-S03 #158".into()],
                            samples: 0,
                            metrics: BTreeMap::new(),
                            counters: Counters::default(),
                        },
                    };
                    ((*name).to_owned(), result)
                })
                .collect(),
        }
    }

    fn baseline(scenarios: &[(&str, f64)]) -> Baseline {
        let mut b = Baseline {
            schema: BASELINE_SCHEMA,
            ..Baseline::default()
        };
        b.os.insert(
            "linux".into(),
            OsBaseline {
                scenarios: scenarios
                    .iter()
                    .map(|(n, v)| {
                        (
                            (*n).to_owned(),
                            [("first_frame_ms".to_owned(), pct(*v))].into(),
                        )
                    })
                    .collect(),
                ..OsBaseline::default()
            },
        );
        b
    }

    #[test]
    fn plus_19_percent_passes() {
        let c = compare(
            &report(&[("startup", Some(119.0))]),
            &baseline(&[("startup", 100.0)]),
            TOL,
            FLOOR,
        );
        assert!(!c.failed(), "{:#?}", c.rows);
        assert_eq!(c.rows.len(), 3, "p50, p95, p99");
        assert!(c.rows.iter().all(|r| r.outcome == Outcome::Pass));
        assert!((c.rows[0].change_pct().unwrap() - 19.0).abs() < 1e-9);
    }

    #[test]
    fn plus_21_percent_fails() {
        let c = compare(
            &report(&[("startup", Some(121.0))]),
            &baseline(&[("startup", 100.0)]),
            TOL,
            FLOOR,
        );
        assert!(c.failed());
        assert!(c.rows.iter().all(|r| r.outcome == Outcome::Regressed));
        assert!(c.rows[0].to_string().starts_with("FAIL"));
    }

    #[test]
    fn exactly_plus_20_percent_passes_and_faster_passes() {
        assert!(!regressed(100.0, 120.0, TOL, FLOOR));
        assert!(!regressed(100.0, 50.0, TOL, FLOOR));
    }

    #[test]
    fn sub_floor_slowdown_passes_but_says_so() {
        let c = compare(
            &report(&[("startup", Some(0.10))]),
            &baseline(&[("startup", 0.05)]),
            TOL,
            FLOOR,
        );
        assert!(!c.failed());
        assert!(
            c.rows
                .iter()
                .all(|r| r.outcome == Outcome::WithinNoiseFloor)
        );
        let line = c.rows[0].to_string();
        assert!(
            line.starts_with("PASS") && line.ends_with("(under the noise floor)"),
            "{line}"
        );
    }

    #[test]
    fn noise_floor_absorbs_microsecond_jitter() {
        // +100 % on a 0.05 ms metric is 0.05 ms: below the 0.25 ms floor.
        assert!(!regressed(0.05, 0.10, TOL, FLOOR));
        assert!(regressed(0.05, 0.40, TOL, FLOOR));
        // With no floor the +21 % rule alone applies.
        assert!(regressed(0.05, 0.0606, TOL, 0.0));
    }

    #[test]
    fn scenario_missing_from_baseline_is_reported_not_fatal() {
        let c = compare(
            &report(&[("startup", Some(10.0)), ("scroll-10k", Some(5.0))]),
            &baseline(&[("startup", 10.0)]),
            TOL,
            FLOOR,
        );
        assert!(!c.failed());
        let row = c.rows.iter().find(|r| r.scenario == "scroll-10k").unwrap();
        let Outcome::MissingBaseline(msg) = &row.outcome else {
            panic!("{row:?}")
        };
        assert!(
            msg.contains("`scroll-10k` has no baseline for os `linux`"),
            "{msg}"
        );
        assert!(row.to_string().starts_with("MISSING"));
    }

    #[test]
    fn os_missing_from_baseline_says_how_to_seed() {
        let mut b = baseline(&[("startup", 10.0)]);
        b.os.clear();
        let c = compare(&report(&[("startup", Some(10.0))]), &b, TOL, FLOOR);
        assert!(!c.failed());
        let Outcome::MissingBaseline(msg) = &c.rows[0].outcome else {
            panic!()
        };
        assert!(msg.contains("no baseline for os `linux`") && msg.contains("--update-baseline"));
    }

    #[test]
    fn baselined_scenario_that_stopped_running_fails() {
        let c = compare(
            &report(&[("palette", None)]),
            &baseline(&[("palette", 3.0)]),
            TOL,
            FLOOR,
        );
        assert!(c.failed());
        let Outcome::MissingInRun(msg) = &c.rows[0].outcome else {
            panic!()
        };
        assert!(msg.contains("not available"));
    }

    #[test]
    fn unavailable_scenario_without_baseline_is_skipped() {
        let c = compare(&report(&[("palette", None)]), &baseline(&[]), TOL, FLOOR);
        assert!(!c.failed());
        let row = &c.rows[0];
        assert!(matches!(row.outcome, Outcome::Skipped(_)));
        assert!(row.to_string().contains("enabled by E11-S03 #158"), "{row}");
    }

    #[test]
    fn scenario_not_requested_is_not_compared() {
        let c = compare(
            &report(&[("startup", Some(10.0))]),
            &baseline(&[("startup", 10.0), ("palette", 3.0)]),
            TOL,
            FLOOR,
        );
        assert!(!c.failed());
        assert!(c.rows.iter().all(|r| r.scenario == "startup"));
    }

    #[test]
    fn update_replaces_this_os_and_keeps_others() {
        let mut b = baseline(&[("startup", 10.0), ("palette", 3.0)]);
        b.os.insert("macos".into(), OsBaseline::default());
        let r = report(&[("startup", Some(12.0)), ("logs-stream", None)]);
        b.update_from(&r, "test", "2026-10-03");
        let linux = &b.os["linux"];
        assert_eq!(linux.scenarios["startup"]["first_frame_ms"].p50, 12.0);
        assert!(
            linux.scenarios.contains_key("palette"),
            "untouched scenario kept"
        );
        assert!(
            !linux.scenarios.contains_key("logs-stream"),
            "unavailable not stored"
        );
        assert_eq!(linux.source, "test");
        assert!(b.os.contains_key("macos"));
    }

    #[test]
    fn committed_baseline_parses() {
        let b: Baseline =
            serde_json::from_str(include_str!("../../../docs/perf/baseline.json")).unwrap();
        assert_eq!(b.schema, BASELINE_SCHEMA);
    }
}
