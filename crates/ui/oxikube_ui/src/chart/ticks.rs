//! Axis tick choice: wall-clock ticks for the time axis, "nice" numbers for the value axis.
//!
//! Pure functions over plain numbers, so every range a chart shows (15 m, 1 h, 6 h, 24 h) is a
//! unit test.

use super::data::TimeAxis;
use super::format::ValueFormat;
use jiff::{
    Timestamp,
    tz::{Offset, TimeZone},
};

/// One labelled tick on the time axis.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeTick {
    /// Seconds since the Unix epoch.
    pub t: f64,
    /// The text under the tick.
    pub label: String,
}

/// Tick spacings, in seconds, that land on wall-clock marks (every 5 minutes, every 3 hours...).
const STEPS: [i64; 19] = [
    1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1_800, 3_600, 7_200, 10_800, 21_600, 43_200,
    86_400, 604_800,
];

/// Time ticks for `axis` drawn `width` pixels wide, each label about `label_width` pixels wide.
///
/// The spacing is the smallest wall-clock step that keeps the labels at least 1.6 label widths
/// apart (never more than 12 ticks). Ticks sit on multiples of the step in the axis' own time
/// zone, so a 6-hour range ticks on the hour and a 24-hour one every three hours. Under a day a
/// midnight tick reads as a date ("Oct 12"), so a range that crosses midnight says so.
pub fn time_ticks(axis: &TimeAxis, width: f32, label_width: f32) -> Vec<TimeTick> {
    let span = axis.span();
    let max_ticks = if width > 0.0 && label_width > 0.0 {
        ((width / (label_width * 1.6)).floor() as usize).clamp(2, 12)
    } else {
        2
    };
    let step = STEPS
        .iter()
        .copied()
        .find(|step| (span / *step as f64).floor() + 1.0 <= max_ticks as f64)
        .unwrap_or(*STEPS.last().unwrap_or(&604_800));
    let offset = f64::from(axis.offset_secs);
    let first = (((axis.start + offset) / step as f64).ceil() as i64) * step;
    let zone = TimeZone::fixed(Offset::from_seconds(axis.offset_secs).unwrap_or(Offset::UTC));
    let mut ticks = Vec::new();
    let mut local = first;
    // Bounded: at most `span / step + 1` iterations, and `span / step` is under 13 by choice.
    while (local as f64) - offset <= axis.end + 1e-6 && ticks.len() < 64 {
        let t = (local as f64) - offset;
        ticks.push(TimeTick {
            t,
            label: tick_label(local, step, &zone, axis.offset_secs),
        });
        local += step;
    }
    ticks
}

fn tick_label(local: i64, step: i64, zone: &TimeZone, offset_secs: i32) -> String {
    let utc = local - i64::from(offset_secs);
    let Ok(ts) = Timestamp::from_second(utc) else {
        return String::new();
    };
    let at = ts.to_zoned(zone.clone());
    if step >= 86_400 || (local.rem_euclid(86_400) == 0) {
        at.strftime("%b %-d").to_string()
    } else if step >= 60 {
        at.strftime("%H:%M").to_string()
    } else {
        at.strftime("%H:%M:%S").to_string()
    }
}

/// The text of a tooltip title: the time to the second, with the date when the range spans days.
pub fn tooltip_time(t: f64, axis: &TimeAxis) -> String {
    let Ok(ts) = Timestamp::from_second(t.floor() as i64) else {
        return String::new();
    };
    let zone = TimeZone::fixed(Offset::from_seconds(axis.offset_secs).unwrap_or(Offset::UTC));
    let at = ts.to_zoned(zone);
    if axis.span() > 86_400.0 {
        at.strftime("%b %-d %H:%M").to_string()
    } else {
        at.strftime("%H:%M:%S").to_string()
    }
}

/// Value-axis ticks.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueTicks {
    /// Tick values from the bottom of the domain to the top, inclusive.
    pub values: Vec<f64>,
    /// The spacing between ticks.
    pub step: f64,
}

impl ValueTicks {
    /// The domain the ticks span.
    pub fn domain(&self) -> (f64, f64) {
        (
            self.values.first().copied().unwrap_or(0.0),
            self.values.last().copied().unwrap_or(1.0),
        )
    }
}

/// "Nice" ticks (steps of 1, 2, 2.5 or 5 times a power of ten, in `unit` multiples) covering
/// `min..=max` with about `target` intervals. The domain is widened to whole steps, so the top
/// gridline sits above the highest value. A degenerate range (`min >= max`) gets a unit interval.
pub fn value_ticks(min: f64, max: f64, target: usize, unit: f64) -> ValueTicks {
    let unit = if unit.is_finite() && unit > 0.0 {
        unit
    } else {
        1.0
    };
    let (lo, hi) = (min / unit, max / unit);
    let (lo, hi) = if hi > lo {
        (lo, hi)
    } else if lo.is_finite() {
        (lo, lo + 1.0)
    } else {
        (0.0, 1.0)
    };
    let raw = (hi - lo) / target.max(1) as f64;
    let magnitude = 10f64.powf(raw.log10().floor());
    let residual = raw / magnitude;
    let nice = if residual <= 1.0 {
        1.0
    } else if residual <= 2.0 {
        2.0
    } else if residual <= 2.5 {
        2.5
    } else if residual <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = nice * magnitude;
    let first = (lo / step).floor();
    let last = (hi / step).ceil();
    let count = ((last - first) as usize).min(32);
    let values = (0..=count)
        .map(|i| (first + i as f64) * step * unit)
        .collect();
    ValueTicks {
        values,
        step: step * unit,
    }
}

/// The labels of `ticks`, written in `format`.
pub fn value_labels(ticks: &ValueTicks, format: &ValueFormat) -> Vec<String> {
    let (lo, hi) = ticks.domain();
    let extent = lo.abs().max(hi.abs());
    ticks
        .values
        .iter()
        .map(|v| format.format_tick(*v, ticks.step, extent))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A midnight-aligned base so wall-clock expectations are readable: 2026-10-12 00:00 UTC.
    const BASE: f64 = 1_791_763_200.0;

    fn labels(axis: &TimeAxis, width: f32) -> Vec<String> {
        time_ticks(axis, width, 40.0)
            .into_iter()
            .map(|t| t.label)
            .collect()
    }

    #[test]
    fn fifteen_minutes_ticks_every_two_minutes() {
        let axis = TimeAxis::utc(BASE + 3600.0, BASE + 3600.0 + 900.0);
        assert_eq!(
            labels(&axis, 600.0),
            [
                "01:00", "01:02", "01:04", "01:06", "01:08", "01:10", "01:12", "01:14"
            ]
        );
    }

    #[test]
    fn one_hour_ticks_every_ten_minutes() {
        let axis = TimeAxis::utc(BASE + 3600.0, BASE + 7200.0);
        assert_eq!(
            labels(&axis, 600.0),
            [
                "01:00", "01:10", "01:20", "01:30", "01:40", "01:50", "02:00"
            ]
        );
    }

    #[test]
    fn six_hours_ticks_every_hour() {
        let axis = TimeAxis::utc(BASE + 6.0 * 3600.0, BASE + 12.0 * 3600.0);
        assert_eq!(
            labels(&axis, 600.0),
            [
                "06:00", "07:00", "08:00", "09:00", "10:00", "11:00", "12:00"
            ]
        );
    }

    #[test]
    fn a_day_ticks_every_three_hours_and_midnight_reads_as_a_date() {
        let axis = TimeAxis::utc(BASE - 12.0 * 3600.0, BASE + 12.0 * 3600.0);
        let ticks = labels(&axis, 600.0);
        assert_eq!(ticks.len(), 9);
        assert_eq!(ticks[0], "12:00");
        assert_eq!(ticks[4], "Oct 12");
        assert_eq!(ticks[8], "12:00");
    }

    #[test]
    fn ticks_follow_the_axis_time_zone() {
        // UTC+9: 00:00 UTC is 09:00 local, and ticks sit on local hours.
        let axis = TimeAxis::new(BASE, BASE + 6.0 * 3600.0, 9 * 3600);
        assert_eq!(labels(&axis, 600.0)[0], "09:00");
        // A half-hour zone ticks on local marks, not UTC ones.
        let india = TimeAxis::new(BASE, BASE + 3600.0, 19_800);
        assert_eq!(labels(&india, 600.0)[0], "05:30");
    }

    #[test]
    fn ticks_stay_inside_the_range_and_are_ordered() {
        for span in [30.0, 900.0, 3600.0, 21_600.0, 86_400.0, 7.0 * 86_400.0] {
            for width in [120.0, 300.0, 600.0, 1400.0] {
                let axis = TimeAxis::utc(BASE + 17.0, BASE + 17.0 + span);
                let ticks = time_ticks(&axis, width, 40.0);
                assert!((1..=12).contains(&ticks.len()), "{span} {width}");
                assert!(ticks.windows(2).all(|w| w[0].t < w[1].t));
                assert!(
                    ticks
                        .iter()
                        .all(|t| t.t >= axis.start - 1e-6 && t.t <= axis.end + 1e-6)
                );
            }
        }
    }

    #[test]
    fn a_week_ticks_by_day() {
        let axis = TimeAxis::utc(BASE, BASE + 7.0 * 86_400.0);
        let ticks = labels(&axis, 600.0);
        assert_eq!(ticks[0], "Oct 12");
        assert_eq!(ticks[1], "Oct 13");
    }

    #[test]
    fn tooltip_titles_add_the_date_for_long_ranges() {
        let short = TimeAxis::utc(BASE, BASE + 3600.0);
        assert_eq!(tooltip_time(BASE + 3723.0, &short), "01:02:03");
        let long = TimeAxis::utc(BASE, BASE + 3.0 * 86_400.0);
        assert_eq!(tooltip_time(BASE + 3723.0, &long), "Oct 12 01:02");
    }

    #[test]
    fn value_ticks_are_nice_and_cover_the_range() {
        let t = value_ticks(0.0, 87.0, 4, 1.0);
        assert_eq!(t.values, [0.0, 25.0, 50.0, 75.0, 100.0]);
        let t = value_ticks(0.0, 1.0, 4, 1.0);
        assert_eq!(t.values, [0.0, 0.25, 0.5, 0.75, 1.0]);
        let t = value_ticks(0.0, 0.0, 4, 1.0);
        assert_eq!(t.domain(), (0.0, 1.0));
        let t = value_ticks(-3.0, 8.0, 4, 1.0);
        assert!(t.domain().0 <= -3.0 && t.domain().1 >= 8.0);
    }

    #[test]
    fn byte_ticks_land_on_binary_marks() {
        let gib = 1024.0f64.powi(3);
        let unit = ValueFormat::Bytes.tick_unit(1.7 * gib);
        assert_eq!(unit, gib);
        let t = value_ticks(0.0, 1.7 * gib, 4, unit);
        assert_eq!(t.values.last().copied(), Some(2.0 * gib));
        let labels = value_labels(&t, &ValueFormat::Bytes);
        assert_eq!(labels.first().map(String::as_str), Some("0 GiB"));
        assert_eq!(labels.last().map(String::as_str), Some("2 GiB"));
    }

    #[test]
    fn millicore_ticks_read_in_millicores() {
        let unit = ValueFormat::Cores.tick_unit(0.4);
        let t = value_ticks(0.0, 0.4, 4, unit);
        let labels = value_labels(&t, &ValueFormat::Cores);
        assert_eq!(labels, ["0", "100m", "200m", "300m", "400m"]);
    }
}
