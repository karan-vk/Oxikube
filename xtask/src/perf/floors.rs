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
    /// `launch_to_first_frame_ms`, `first_rows_ms`, `init_window_ms`): a whole launch, so one
    /// slow runner moves them by tens of milliseconds with unchanged code.
    ColdStart,
    /// `frame_ms` / `draw_ms` and every `<mode>_frame_ms` / `<mode>_draw_ms`: the p50/p95/p99 of
    /// many frames in one process. Real views put these at 1 to 5 ms, so a small floor lets them
    /// gate.
    Frame,
    /// Any other `*_ms` metric: the small startup stages (`config_load_ms`, `state_db_open_ms`,
    /// `init_<stage>_ms`).
    Stage,
}

impl MetricClass {
    /// The class of `metric`, from its name.
    pub fn of(metric: &str) -> Self {
        if metric_unit(metric) == "MiB" {
            Self::Memory
        } else if metric.ends_with("first_frame_ms")
            || metric.ends_with("first_rows_ms")
            || metric == "init_window_ms"
        {
            Self::ColdStart
        } else if metric.ends_with("frame_ms") || metric.ends_with("draw_ms") {
            Self::Frame
        } else {
            Self::Stage
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
            MetricClass::Frame | MetricClass::Stage => self.ms,
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
            ("frame_ms", Frame),
            ("draw_ms", Frame),
            ("paused_frame_ms", Frame),
            ("wrap_paused_draw_ms", Frame),
            ("merged_wrap_frame_ms", Frame),
            ("first_frame_ms", ColdStart),
            ("launch_to_first_frame_ms", ColdStart),
            ("first_rows_ms", ColdStart),
            ("init_window_ms", ColdStart),
            ("config_load_ms", Stage),
            ("state_db_open_ms", Stage),
            ("init_ui_ms", Stage),
            ("init_assets_ms", Stage),
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
