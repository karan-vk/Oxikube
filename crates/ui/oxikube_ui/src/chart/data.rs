//! Plain chart data: samples, series, the time axis, thresholds and bands.
//!
//! Nothing here depends on GPUI contexts, the domain crate or gpui-component, so the metrics
//! views build these from their own types and the pure parts (decimation, ticks, formatting) are
//! unit-testable.

use gpui::{Hsla, SharedString};
use std::sync::Arc;

/// One sample: seconds since the Unix epoch (fractions allowed) and a value.
///
/// A non-finite value (`NaN`, infinities) is a **gap**: the line breaks there instead of being
/// drawn to zero. A sample whose time is not finite is dropped by [`Series::new`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Seconds since the Unix epoch.
    pub t: f64,
    /// The value; non-finite means "no reading".
    pub v: f64,
}

impl Point {
    /// A sample at `t` seconds with value `v`.
    pub const fn new(t: f64, v: f64) -> Self {
        Self { t, v }
    }

    /// A gap marker at `t`.
    pub const fn gap(t: f64) -> Self {
        Self { t, v: f64::NAN }
    }

    /// Whether this sample has no reading.
    pub fn is_gap(&self) -> bool {
        !self.v.is_finite()
    }
}

/// A semantic colour: resolved against the active theme, so a series follows light and dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Tone {
    /// Within bounds (the success colour).
    Good,
    /// Approaching a limit (the warning colour); the colour of a warn band or threshold.
    Warn,
    /// Over a limit (the error colour); the colour of a critical band or threshold.
    Critical,
    /// Neutral information (the info colour).
    Info,
    /// De-emphasised (the muted text colour); the default for a threshold line.
    #[default]
    Muted,
}

/// Where a series takes its colour from.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum SeriesColor {
    /// The next colour of the categorical palette, by the series' position in the chart (so
    /// hiding a series does not recolour the others).
    #[default]
    Auto,
    /// A status colour.
    Tone(Tone),
    /// An explicit colour (prefer the other two: they follow the theme).
    Fixed(Hsla),
}

/// How a series is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeriesStyle {
    /// The colour source.
    pub color: SeriesColor,
    /// Stroke width in design pixels (zoom is applied when drawing).
    pub width: f32,
}

impl Default for SeriesStyle {
    fn default() -> Self {
        Self {
            color: SeriesColor::Auto,
            width: 1.5,
        }
    }
}

/// One named line, area or bar group. Samples must be in ascending time order; [`Series::new`]
/// sorts them when they are not (one `O(n)` check per construction).
///
/// The samples are shared (`Arc`), so handing the same series to a chart on every metrics tick
/// costs a reference count, and the chart redraws from its cached geometry until the data changes.
#[derive(Clone, Debug)]
pub struct Series {
    /// The legend and tooltip name ("nginx", "CPU").
    pub name: SharedString,
    /// The samples, ascending by time.
    pub points: Arc<[Point]>,
    /// Colour and stroke width.
    pub style: SeriesStyle,
}

impl Series {
    /// A series named `name` with default style. Drops samples with a non-finite time and sorts
    /// the rest by time when needed.
    pub fn new(name: impl Into<SharedString>, points: impl Into<Arc<[Point]>>) -> Self {
        let points: Arc<[Point]> = points.into();
        let ordered = points.iter().all(|p| p.t.is_finite())
            && points.windows(2).all(|pair| pair[0].t <= pair[1].t);
        let points = if ordered {
            points
        } else {
            let mut kept: Vec<Point> = points.iter().copied().filter(|p| p.t.is_finite()).collect();
            kept.sort_by(|a, b| a.t.total_cmp(&b.t));
            kept.into()
        };
        Self {
            name: name.into(),
            points,
            style: SeriesStyle::default(),
        }
    }

    /// Replaces the style.
    #[must_use]
    pub fn style(mut self, style: SeriesStyle) -> Self {
        self.style = style;
        self
    }

    /// Colours the series with a status tone.
    #[must_use]
    pub fn tone(mut self, tone: Tone) -> Self {
        self.style.color = SeriesColor::Tone(tone);
        self
    }

    /// Whether the series has at least one reading (a finite value).
    pub fn has_data(&self) -> bool {
        self.points.iter().any(|p| !p.is_gap())
    }

    /// The last reading, skipping trailing gaps.
    pub fn latest(&self) -> Option<Point> {
        self.points.iter().rev().find(|p| !p.is_gap()).copied()
    }
}

/// The visible time range, and the UTC offset its tick labels are written in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeAxis {
    /// Left edge, seconds since the Unix epoch.
    pub start: f64,
    /// Right edge, seconds since the Unix epoch. Always after [`TimeAxis::start`].
    pub end: f64,
    /// Seconds east of UTC the labels are shown in.
    pub offset_secs: i32,
}

impl TimeAxis {
    /// An axis from `start` to `end` with labels at `offset_secs` east of UTC. An empty or
    /// reversed range is widened to one second so scales never divide by zero.
    pub fn new(start: f64, end: f64, offset_secs: i32) -> Self {
        let (start, end) = if start.is_finite() && end.is_finite() && end > start {
            (start, end)
        } else if start.is_finite() {
            (start, start + 1.0)
        } else {
            (0.0, 1.0)
        };
        Self {
            start,
            end,
            offset_secs,
        }
    }

    /// An axis in UTC (tests, exports).
    pub fn utc(start: f64, end: f64) -> Self {
        Self::new(start, end, 0)
    }

    /// An axis whose labels use the machine's local time zone at `start`.
    pub fn local(start: f64, end: f64) -> Self {
        let offset = local_offset_secs(start);
        Self::new(start, end, offset)
    }

    /// The last `seconds` before `now` (the "last 15 minutes" range), in local time.
    pub fn last(seconds: f64, now: f64) -> Self {
        Self::local(now - seconds.max(1.0), now)
    }

    /// An axis covering every sample of `series` (the range of data on hand), in local time.
    pub fn fit(series: &[Series]) -> Self {
        let mut bounds: Option<(f64, f64)> = None;
        for s in series {
            if let (Some(first), Some(last)) = (s.points.first(), s.points.last()) {
                bounds = Some(match bounds {
                    Some((lo, hi)) => (lo.min(first.t), hi.max(last.t)),
                    None => (first.t, last.t),
                });
            }
        }
        let (lo, hi) = bounds.unwrap_or((0.0, 1.0));
        Self::local(lo, hi)
    }

    /// The length of the range in seconds.
    pub fn span(&self) -> f64 {
        self.end - self.start
    }
}

/// The machine's UTC offset in seconds at `t`. Falls back to UTC when the zone is unknown.
fn local_offset_secs(t: f64) -> i32 {
    if !t.is_finite() {
        return 0;
    }
    jiff::Timestamp::from_second(t as i64)
        .map(|ts| jiff::tz::TimeZone::system().to_offset(ts).seconds())
        .unwrap_or(0)
}

/// A horizontal reference line: a request or limit.
#[derive(Clone, Debug, PartialEq)]
pub struct Threshold {
    /// The value, in the chart's value unit.
    pub value: f64,
    /// Drawn at the right end of the line ("limit").
    pub label: SharedString,
    /// The line colour.
    pub tone: Tone,
}

impl Threshold {
    /// A threshold at `value` with a label, drawn in the muted tone.
    pub fn new(value: f64, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
            tone: Tone::Muted,
        }
    }

    /// Sets the line colour.
    #[must_use]
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
}

/// A horizontal band of the value axis shaded in a status tone (the warn and critical zones).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    /// Lower edge, in the chart's value unit.
    pub from: f64,
    /// Upper edge.
    pub to: f64,
    /// The shade.
    pub tone: Tone,
}

impl Band {
    /// A band between `from` and `to`.
    pub fn new(from: f64, to: f64, tone: Tone) -> Self {
        Self { from, to, tone }
    }
}

/// Which way the value axis is scaled.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum YRange {
    /// From zero (or the data minimum when something is negative) to a nice number above the
    /// largest visible value or threshold.
    #[default]
    Auto,
    /// Fixed bounds, such as `0..=100` for a percentage.
    Fixed(f64, f64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_sorts_and_drops_samples_with_bad_times() {
        let s = Series::new(
            "cpu",
            vec![
                Point::new(3.0, 1.0),
                Point::new(f64::NAN, 9.0),
                Point::new(1.0, 2.0),
            ],
        );
        let times: Vec<f64> = s.points.iter().map(|p| p.t).collect();
        assert_eq!(times, [1.0, 3.0]);
    }

    #[test]
    fn ordered_series_keeps_its_allocation() {
        let points: Arc<[Point]> = vec![Point::new(1.0, 1.0), Point::new(2.0, 2.0)].into();
        let s = Series::new("x", points.clone());
        assert!(Arc::ptr_eq(&s.points, &points));
    }

    #[test]
    fn gaps_are_not_readings() {
        let s = Series::new("x", vec![Point::new(1.0, 5.0), Point::gap(2.0)]);
        assert!(s.has_data());
        assert_eq!(s.latest(), Some(Point::new(1.0, 5.0)));
        assert!(!Series::new("y", vec![Point::gap(1.0)]).has_data());
        assert!(!Series::new("z", Vec::<Point>::new()).has_data());
    }

    #[test]
    fn axis_never_has_an_empty_range() {
        assert_eq!(TimeAxis::utc(10.0, 10.0).span(), 1.0);
        assert_eq!(TimeAxis::utc(10.0, 5.0).span(), 1.0);
        assert_eq!(TimeAxis::utc(f64::NAN, 5.0).span(), 1.0);
        assert_eq!(TimeAxis::utc(0.0, 900.0).span(), 900.0);
    }

    #[test]
    fn fit_covers_every_series() {
        let a = Series::new("a", vec![Point::new(100.0, 1.0), Point::new(200.0, 1.0)]);
        let b = Series::new("b", vec![Point::new(50.0, 1.0), Point::new(150.0, 1.0)]);
        let axis = TimeAxis::fit(&[a, b]);
        assert_eq!((axis.start, axis.end), (50.0, 200.0));
    }
}
