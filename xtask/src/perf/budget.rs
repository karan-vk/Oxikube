//! Absolute budgets (ADR 0013, docs/PERFORMANCE.md "Budgets") checked on every perf run, next to
//! the relative baseline gate.
//!
//! The baseline gate catches regressions against the same runner; a budget catches a number that
//! is simply too high, whatever it was yesterday. Each [`Budget`] names a scenario metric, its
//! limit and how much it may exceed it before the run fails. The value checked is the p95 across
//! the cold launches of the run (`ScenarioResult::launches`) when the metric is observed once per
//! process, else the median p95.
//!
//! Headless numbers are a lower bound of the windowed app's (no present, no GPU), so passing here
//! is necessary, not sufficient: the windowed figure comes from `oxikube --perf`.

use super::report::{Report, Status};
use std::fmt;

/// One budget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    pub scenario: &'static str,
    pub metric: &'static str,
    /// The budget, in the metric's unit.
    pub limit: f64,
    /// How far over the limit the run may go before it fails (0.20 = +20 %); between the limit and
    /// that, the row is `OVER` and the run passes.
    pub tolerance: f64,
    /// What the budget is, for the table.
    pub what: &'static str,
    /// Only check on this OS (`std::env::consts::OS`); `None`: everywhere.
    pub os: Option<&'static str>,
}

/// Every budget, in report order.
pub const BUDGETS: &[Budget] = &[
    Budget {
        scenario: "startup",
        metric: "launch_to_first_frame_ms",
        limit: 400.0,
        // The nightly tolerance: the scenario fails when the first frame is more than 20 % over.
        tolerance: 0.20,
        what: "cold start to the first interactive frame (process spawn, headless)",
        os: None,
    },
    Budget {
        scenario: "startup",
        metric: "config_load_ms",
        limit: 30.0,
        // The strict bound (test builds get a generous one): settings, theme and keymap on the
        // main thread.
        tolerance: 0.0,
        what: "settings + theme + keymap load on the main thread",
        os: None,
    },
    Budget {
        scenario: "scroll-10k",
        metric: "first_rows_ms",
        limit: 1000.0,
        tolerance: 0.0,
        what: "first table rows after the feed is warm (10 000 pods, headless)",
        os: None,
    },
    Budget {
        scenario: "scroll-10k",
        metric: "frame_ms",
        // 55 fps (E07's acceptance): a frame of the table scrolling under churn, headless. The
        // p95 <= 8 ms budget is the windowed `oxikube --perf` figure (docs/PERFORMANCE.md).
        limit: 1000.0 / 55.0,
        tolerance: 0.0,
        what: ">= 55 fps scrolling 10 000 pods under churn (headless frame, p95)",
        // macOS only: the Linux runner draws these frames with Mesa's software renderer, about
        // 340 ms each, which says nothing about the app on a GPU (#509). The baseline gate still
        // catches a regression there.
        os: Some("macos"),
    },
    logs_frame(
        "frame_ms",
        "streaming 5 000 lines/s, wrap off, following (headless frame, p95)",
    ),
    logs_frame(
        "paused_frame_ms",
        "streaming 5 000 lines/s, wrap off, autoscroll paused (headless frame, p95)",
    ),
    logs_frame(
        "wrap_frame_ms",
        "streaming 5 000 lines/s, wrapped, following (headless frame, p95)",
    ),
    logs_frame(
        "wrap_paused_frame_ms",
        "streaming 5 000 lines/s, wrapped, autoscroll paused (headless frame, p95)",
    ),
    logs_frame(
        "raw_frame_ms",
        "streaming 5 000 lines/s, JSON mode off, following (headless frame, p95)",
    ),
    logs_frame(
        "json_filtered_frame_ms",
        "streaming 5 000 lines/s of JSON, debug and plain chips off, following (headless frame, p95)",
    ),
    logs_frame(
        "search_frame_ms",
        "streaming 5 000 lines/s with a search highlighting, wrap off, following (headless frame, p95)",
    ),
    logs_frame(
        "filter_frame_ms",
        "streaming 5 000 lines/s with a search filtering, wrap off, following (headless frame, p95)",
    ),
    logs_frame(
        "merged_frame_ms",
        "5 000 lines/s merged from 10 pods, wrap off, following (headless frame, p95)",
    ),
    logs_frame(
        "merged_wrap_frame_ms",
        "5 000 lines/s merged from 10 pods, wrapped, following (headless frame, p95)",
    ),
];

/// A `logs-stream` frame budget (E08-S02, the JSON modes E08-S05, the search modes E08-S03, the merged modes E08-S04):
/// p95 <= 8 ms. macOS only, as the table's (#509): the Linux runner's software renderer says
/// nothing about the app on a GPU.
const fn logs_frame(metric: &'static str, what: &'static str) -> Budget {
    Budget {
        scenario: "logs-stream",
        metric,
        limit: 8.0,
        tolerance: 0.0,
        what,
        os: Some("macos"),
    }
}

/// How a budget fared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Within,
    /// Over the limit, inside the tolerance.
    Over,
    Fail,
    /// The scenario did not run or did not report the metric.
    Missing,
}

/// One row of the budget table.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetRow {
    pub budget: Budget,
    /// The p95 checked, when there was one.
    pub value: Option<f64>,
    pub verdict: Verdict,
}

impl fmt::Display for BudgetRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.budget;
        let verdict = match self.verdict {
            Verdict::Within => "ok",
            Verdict::Over => "OVER",
            Verdict::Fail => "FAIL",
            Verdict::Missing => "MISSING",
        };
        let value = self
            .value
            .map(|v| format!("{v:.3}"))
            .unwrap_or_else(|| "-".into());
        write!(
            f,
            "{verdict:<7} {}/{} p95 {value} (budget {}, fail above {:.1}): {}",
            b.scenario,
            b.metric,
            b.limit,
            b.limit * (1.0 + b.tolerance),
            b.what
        )
    }
}

/// Checks `budgets` against `report`. Scenarios the report did not run are left out.
pub fn check(report: &Report, budgets: &[Budget]) -> Vec<BudgetRow> {
    budgets
        .iter()
        .filter(|budget| budget.os.is_none_or(|os| os == report.os))
        .filter_map(|budget| {
            let result = report.scenarios.get(budget.scenario)?;
            if result.status != Status::Ok {
                return None;
            }
            let value = result
                .launches
                .get(budget.metric)
                .or_else(|| result.metrics.get(budget.metric))
                .map(|p| p.p95);
            let verdict = match value {
                None => Verdict::Missing,
                Some(v) if v <= budget.limit => Verdict::Within,
                Some(v) if v <= budget.limit * (1.0 + budget.tolerance) => Verdict::Over,
                Some(_) => Verdict::Fail,
            };
            Some(BudgetRow {
                budget: *budget,
                value,
                verdict,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::report::{Counters, Percentiles, ScenarioResult};
    use super::*;
    use std::collections::BTreeMap;

    fn pct(v: f64) -> Percentiles {
        Percentiles {
            p50: v,
            p95: v,
            p99: v,
            max: Some(v),
        }
    }

    fn report(first_frame: Option<f64>, config: f64) -> Report {
        let mut launches = BTreeMap::new();
        if let Some(v) = first_frame {
            launches.insert("launch_to_first_frame_ms".to_owned(), pct(v));
        }
        launches.insert("config_load_ms".to_owned(), pct(config));
        let startup = ScenarioResult {
            status: Status::Ok,
            reason: None,
            enabled_by: vec![],
            samples: 20,
            metrics: BTreeMap::new(),
            counters: Counters::default(),
            launches,
        };
        Report {
            schema: 1,
            os: "macos".into(),
            arch: "aarch64".into(),
            profile: "release-fast".into(),
            samples_per_scenario: 20,
            note: String::new(),
            scenarios: [("startup".to_owned(), startup)].into(),
        }
    }

    fn verdicts(report: &Report) -> Vec<Verdict> {
        check(report, BUDGETS).iter().map(|r| r.verdict).collect()
    }

    #[test]
    fn the_first_frame_fails_only_beyond_the_tolerance() {
        assert_eq!(
            verdicts(&report(Some(390.0), 5.0)),
            [Verdict::Within, Verdict::Within]
        );
        assert_eq!(verdicts(&report(Some(470.0), 5.0))[0], Verdict::Over);
        assert_eq!(verdicts(&report(Some(480.5), 5.0))[0], Verdict::Fail);
    }

    #[test]
    fn the_config_load_budget_is_strict() {
        assert_eq!(verdicts(&report(Some(100.0), 30.0))[1], Verdict::Within);
        assert_eq!(verdicts(&report(Some(100.0), 30.5))[1], Verdict::Fail);
    }

    #[test]
    fn the_table_budgets_check_first_rows_and_the_55_fps_frame() {
        let mut r = report(Some(100.0), 1.0);
        let mut launches = BTreeMap::new();
        launches.insert("first_rows_ms".to_owned(), pct(240.0));
        let mut metrics = BTreeMap::new();
        metrics.insert("frame_ms".to_owned(), pct(6.0));
        r.scenarios.insert(
            "scroll-10k".to_owned(),
            ScenarioResult {
                status: Status::Ok,
                reason: None,
                enabled_by: vec![],
                samples: 5,
                metrics,
                counters: Counters::default(),
                launches,
            },
        );
        assert_eq!(verdicts(&r)[2..], [Verdict::Within, Verdict::Within]);
        let scroll = r.scenarios.get_mut("scroll-10k").unwrap();
        scroll
            .launches
            .insert("first_rows_ms".to_owned(), pct(1000.5));
        scroll.metrics.insert("frame_ms".to_owned(), pct(18.5));
        assert_eq!(verdicts(&r)[2..], [Verdict::Fail, Verdict::Fail]);
        // The frame budget is macOS only (#509).
        r.os = "linux".into();
        assert_eq!(verdicts(&r)[2..], [Verdict::Fail]);
    }

    #[test]
    fn the_log_budgets_hold_every_mode_to_an_8_ms_p95() {
        let mut r = report(Some(100.0), 1.0);
        let modes = [
            "frame_ms",
            "paused_frame_ms",
            "wrap_frame_ms",
            "wrap_paused_frame_ms",
            "raw_frame_ms",
            "json_filtered_frame_ms",
            "search_frame_ms",
            "filter_frame_ms",
            "merged_frame_ms",
            "merged_wrap_frame_ms",
        ];
        let metrics = modes.iter().map(|m| ((*m).to_owned(), pct(4.0))).collect();
        r.scenarios.insert(
            "logs-stream".to_owned(),
            ScenarioResult {
                status: Status::Ok,
                reason: None,
                enabled_by: vec![],
                samples: 5,
                metrics,
                counters: Counters::default(),
                launches: BTreeMap::new(),
            },
        );
        assert_eq!(verdicts(&r)[2..], [Verdict::Within; 10]);
        let logs = r.scenarios.get_mut("logs-stream").unwrap();
        logs.metrics.insert("wrap_frame_ms".to_owned(), pct(8.1));
        assert_eq!(verdicts(&r)[4], Verdict::Fail);
        r.os = "linux".into();
        assert_eq!(verdicts(&r).len(), 2, "macOS only");
    }

    #[test]
    fn a_missing_metric_is_reported_and_other_scenarios_are_skipped() {
        assert_eq!(verdicts(&report(None, 1.0))[0], Verdict::Missing);
        let mut empty = report(None, 1.0);
        empty.scenarios.clear();
        assert!(check(&empty, BUDGETS).is_empty());
    }
}
