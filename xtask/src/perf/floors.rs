//! The absolute noise floors of `cargo xtask perf --check` (docs/PERFORMANCE.md, "Baseline and
//! the nightly gate"): how much slower a metric must be, besides the relative tolerance, before
//! it counts as a regression. One floor per class of metric, because the classes have different
//! run-to-run spread on a shared runner.

use super::report::metric_unit;

/// Which floor applies to a metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricClass {
    /// `*_mib`: resident memory.
    Memory,
    /// Cold-start milestones, observed once per process (`first_frame_ms`,
    /// `launch_to_first_frame_ms`, `first_rows_ms`, `init_window_ms`, and the editor's 5 MB
    /// `open_ms` and `validation_ms`, E10-S11): a whole launch, or hundreds of milliseconds of
    /// work measured once, so one slow runner moves them by tens of milliseconds with unchanged
    /// code.
    ColdStart,
    /// `*_pct`: a share of frames (`slow_frame_pct`, E10-S11), compared with [`PERCENT_FLOOR`].
    Percent,
    /// Every other `*_ms` metric: per-frame percentiles (`frame_ms`, `<mode>_draw_ms`), which
    /// real views put at 1 to 5 ms, and the small startup stages (`config_load_ms`,
    /// `init_<stage>_ms`). A small floor lets them gate.
    Millis,
}

/// Percentage points a `*_pct` metric must also grow by to fail: one point is more than one
/// frame in a hundred, so a single slow frame on a noisy runner does not fail the gate (the
/// absolute budget still applies).
pub const PERCENT_FLOOR: f64 = 1.0;

impl MetricClass {
    /// The class of `metric`, from its name.
    pub fn of(metric: &str) -> Self {
        if metric_unit(metric) == "MiB" {
            Self::Memory
        } else if metric_unit(metric) == "%" {
            Self::Percent
        } else if metric.ends_with("first_frame_ms")
            || metric.ends_with("first_rows_ms")
            || metric == "init_window_ms"
            || metric == "open_ms"
            || metric == "validation_ms"
        {
            Self::ColdStart
        } else {
            Self::Millis
        }
    }
}

/// Absolute slowdown (or growth) a metric must also exceed to fail, per class of metric, in the
/// metric's unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseFloors {
    /// Frame and stage `*_ms` metrics (`--noise-floor-ms`).
    pub ms: f64,
    /// Cold-start milestones (`--noise-floor-cold-ms`).
    pub cold_ms: f64,
    /// `*_mib` metrics (`--noise-floor-mib`).
    pub mib: f64,
}

impl NoiseFloors {
    /// The floor that applies to `metric`.
    pub fn for_metric(&self, metric: &str) -> f64 {
        match MetricClass::of(metric) {
            MetricClass::Memory => self.mib,
            MetricClass::ColdStart => self.cold_ms,
            MetricClass::Millis => self.ms,
            MetricClass::Percent => PERCENT_FLOOR,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::baseline::regressed;
    use super::*;

    const FLOORS: NoiseFloors = NoiseFloors {
        ms: 0.25,
        cold_ms: 40.0,
        mib: 8.0,
    };

    #[test]
    fn metrics_are_classified_by_name() {
        use MetricClass::*;
        for (metric, class) in [
            ("rss_mib", Memory),
            ("peak_rss_mib", Memory),
            ("frame_ms", Millis),
            ("draw_ms", Millis),
            ("paused_frame_ms", Millis),
            ("wrap_paused_draw_ms", Millis),
            ("merged_wrap_frame_ms", Millis),
            ("first_frame_ms", ColdStart),
            ("launch_to_first_frame_ms", ColdStart),
            ("first_rows_ms", ColdStart),
            ("init_window_ms", ColdStart),
            ("config_load_ms", Millis),
            ("state_db_open_ms", Millis),
            ("init_ui_ms", Millis),
            ("init_assets_ms", Millis),
            ("open_ms", ColdStart),
            ("validation_ms", ColdStart),
            ("scroll_frame_ms", Millis),
            ("slow_frame_pct", Percent),
        ] {
            assert_eq!(MetricClass::of(metric), class, "{metric}");
        }
    }

    #[test]
    fn floors_apply_per_class() {
        assert_eq!(FLOORS.for_metric("rss_mib"), 8.0);
        assert_eq!(FLOORS.for_metric("peak_rss_mib"), 8.0);
        assert_eq!(FLOORS.for_metric("frame_ms"), 0.25);
        assert_eq!(FLOORS.for_metric("search_draw_ms"), 0.25);
        assert_eq!(FLOORS.for_metric("config_load_ms"), 0.25);
        assert_eq!(FLOORS.for_metric("first_frame_ms"), 40.0);
        assert_eq!(FLOORS.for_metric("launch_to_first_frame_ms"), 40.0);
        assert_eq!(FLOORS.for_metric("first_rows_ms"), 40.0);
        assert_eq!(FLOORS.for_metric("open_ms"), 40.0);
        assert_eq!(FLOORS.for_metric("slow_frame_pct"), PERCENT_FLOOR);
    }

    /// #411: the nightly of an unchanged tree read 95.7 -> 120.3 ms (+25.7 %) on one slow runner
    /// and failed the 20 % gate. The cold-start floor lets that through; a real +60 ms doubling
    /// still fails.
    #[test]
    fn one_slow_runner_does_not_fail_a_cold_start_metric() {
        let floor = FLOORS.for_metric("first_frame_ms");
        assert!(!regressed(95.7, 120.3, 0.20, floor));
        assert!(!regressed(
            97.9,
            122.8,
            0.20,
            FLOORS.for_metric("launch_to_first_frame_ms")
        ));
        assert!(regressed(95.7, 160.0, 0.20, floor));
    }

    /// #411: with real views the frame metrics are in the millisecond range and gate. A
    /// 1.6 ms log frame becoming 8 ms (the budget) fails; the 0.01 ms placeholder case that
    /// motivated the issue no longer defines the floor.
    #[test]
    fn frame_metrics_gate_in_the_millisecond_range() {
        let floor = FLOORS.for_metric("frame_ms");
        assert!(regressed(1.61, 8.0, 0.20, floor));
        assert!(regressed(3.17, 4.0, 0.20, floor));
        assert!(!regressed(3.17, 3.3, 0.20, floor), "jitter");
    }
}
