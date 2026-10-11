//! The step and `rate()` window of a range: one policy the catalogue and the views share.

use std::time::Duration;

/// The most points a range query asks for. Fewer points than a chart has pixels, more than a
/// line needs to look continuous; it also bounds the response size.
pub const MAX_POINTS: u64 = 300;

/// The smallest step, so a short range never asks for finer data than the usual scrape
/// interval can give.
pub const MIN_STEP: Duration = Duration::from_secs(15);

/// The smallest `rate()` window: four scrapes of the usual 30 s interval (the rule behind
/// Grafana's `$__rate_interval`), so a missed scrape does not empty the graph.
pub const MIN_RATE_WINDOW: Duration = Duration::from_secs(120);

/// The `rate()` window of an instant query ("the value now").
pub const INSTANT_RATE_WINDOW: Duration = Duration::from_secs(300);

/// Steps that read well on an axis and align with scrape intervals, ascending.
const LADDER_SECS: [u64; 9] = [15, 30, 60, 120, 300, 600, 900, 1800, 3600];

/// The step of a range and the `rate()` window to use with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeSteps {
    /// Distance between points.
    pub step: Duration,
    /// Window of every `rate()` in the query: wide enough for [`MIN_RATE_WINDOW`] and two
    /// steps, so neighbouring points overlap and the line is smooth but not smeared.
    pub rate_window: Duration,
}

/// The step policy for a range of `span`: the smallest readable step (15 s, 30 s, 1 m, 2 m,
/// 5 m, ... 1 h) that keeps the range within [`MAX_POINTS`] points (never below [`MIN_STEP`]); the `rate()` window is
/// `max(2 x step, MIN_RATE_WINDOW)`.
///
/// 15 m gives 15 s / 2 m, 1 h 15 s / 2 m, 6 h 2 m / 4 m and 24 h 5 m / 10 m.
pub fn steps_for(span: Duration) -> RangeSteps {
    let wanted = span.as_secs().div_ceil(MAX_POINTS).max(MIN_STEP.as_secs());
    let secs = LADDER_SECS
        .iter()
        .copied()
        .find(|&s| s >= wanted)
        .unwrap_or_else(|| wanted.div_ceil(3600) * 3600);
    let step = Duration::from_secs(secs);
    RangeSteps {
        step,
        rate_window: (step * 2).max(MIN_RATE_WINDOW),
    }
}

/// The ranges a detail panel offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangePreset {
    /// The last 15 minutes.
    Minutes15,
    /// The last hour (the default).
    Hour1,
    /// The last 6 hours.
    Hours6,
    /// The last 24 hours.
    Hours24,
}

impl RangePreset {
    /// Every preset, shortest first.
    pub const ALL: [RangePreset; 4] = [
        RangePreset::Minutes15,
        RangePreset::Hour1,
        RangePreset::Hours6,
        RangePreset::Hours24,
    ];

    /// The preset a panel opens with.
    pub const DEFAULT: RangePreset = RangePreset::Hour1;

    /// How far back the range reaches.
    pub const fn span(self) -> Duration {
        Duration::from_secs(match self {
            RangePreset::Minutes15 => 15 * 60,
            RangePreset::Hour1 => 60 * 60,
            RangePreset::Hours6 => 6 * 60 * 60,
            RangePreset::Hours24 => 24 * 60 * 60,
        })
    }

    /// The label of the selector segment (`15m`).
    pub const fn label(self) -> &'static str {
        match self {
            RangePreset::Minutes15 => "15m",
            RangePreset::Hour1 => "1h",
            RangePreset::Hours6 => "6h",
            RangePreset::Hours24 => "24h",
        }
    }

    /// The step and window for this preset.
    pub fn steps(self) -> RangeSteps {
        steps_for(self.span())
    }
}

/// A duration as PromQL writes it: the largest whole unit (`2m`, `1h`, `90s`). Sub-second
/// remainders are dropped, and zero is `0s`.
pub fn promql_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs != 0 && secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs != 0 && secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn the_four_presets_get_the_documented_step_and_window() {
        let expected = [
            (RangePreset::Minutes15, 15, 120),
            (RangePreset::Hour1, 15, 120),
            (RangePreset::Hours6, 120, 240),
            (RangePreset::Hours24, 300, 600),
        ];
        for (preset, step, window) in expected {
            let steps = preset.steps();
            assert_eq!(steps.step, secs(step), "{}", preset.label());
            assert_eq!(steps.rate_window, secs(window), "{}", preset.label());
        }
    }

    #[test]
    fn a_preset_never_asks_for_more_than_the_point_budget() {
        for preset in RangePreset::ALL {
            let points = preset.span().as_secs() / preset.steps().step.as_secs();
            assert!(points <= MAX_POINTS, "{}: {points}", preset.label());
            assert!(points >= 50, "{}: {points}", preset.label());
        }
    }

    #[test]
    fn long_and_tiny_ranges_stay_within_bounds() {
        assert_eq!(steps_for(secs(1)).step, MIN_STEP);
        assert_eq!(steps_for(Duration::ZERO).step, MIN_STEP);
        let week = steps_for(secs(7 * 86_400));
        assert_eq!(week.step, secs(3600));
        let year = steps_for(secs(365 * 86_400));
        assert_eq!(year.step.as_secs() % 3600, 0);
        assert!(365 * 86_400 / year.step.as_secs() <= MAX_POINTS);
    }

    #[test]
    fn the_step_never_shrinks_as_the_range_grows() {
        let mut last = Duration::ZERO;
        for minutes in 1..=3000 {
            let step = steps_for(secs(minutes * 60)).step;
            assert!(step >= last, "{minutes} min");
            last = step;
        }
    }

    #[test]
    fn durations_use_the_largest_whole_unit() {
        assert_eq!(promql_duration(secs(90)), "90s");
        assert_eq!(promql_duration(secs(120)), "2m");
        assert_eq!(promql_duration(secs(300)), "5m");
        assert_eq!(promql_duration(secs(3600)), "1h");
        assert_eq!(promql_duration(secs(5400)), "90m");
        assert_eq!(promql_duration(Duration::ZERO), "0s");
    }

    #[test]
    fn the_default_preset_is_one_hour() {
        assert_eq!(RangePreset::DEFAULT.span(), secs(3600));
    }
}
