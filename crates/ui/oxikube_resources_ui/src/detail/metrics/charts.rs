//! From a fetched [`ChartData`] to the chart entities: series, the axis, reference lines and the
//! unit's format. No I/O; the entities are updated in place so a refresh repaints from cache when
//! nothing changed.

use gpui::{AppContext as _, Context, Entity};
use oxikube_app::metrics_history::{ReferenceKind, ReferenceLine, SeriesData};
use oxikube_app::{ChartData, ChartDef};
use oxikube_domain::MetricUnit;
use oxikube_ui::chart::{Chart, Point, Series, Threshold, TimeAxis, Tone, ValueFormat};

use crate::detail::view::DetailView;

/// Which chart a slot holds: the id from the definition, which is also its element id.
pub(in crate::detail) type ChartKey = &'static str;

/// The axis format of a unit.
pub(super) fn format_of(unit: MetricUnit) -> ValueFormat {
    match unit {
        MetricUnit::Cores => ValueFormat::Cores,
        MetricUnit::Bytes => ValueFormat::Bytes,
        MetricUnit::BytesPerSecond => ValueFormat::BytesPerSecond,
        MetricUnit::Count => ValueFormat::Number,
    }
}

/// One series as chart points. A sample without a reading becomes a gap, so the line breaks
/// instead of dropping to zero.
pub(super) fn series_of(data: &SeriesData) -> Series {
    let points: Vec<Point> = data
        .samples
        .iter()
        .map(|s| {
            if s.v.is_finite() {
                Point::new(s.t, s.v)
            } else {
                Point::gap(s.t)
            }
        })
        .collect();
    Series::new(data.name.clone(), points)
}

/// A reference line as a threshold: limits in the critical tone, requests and capacity muted.
pub(super) fn threshold_of(line: &ReferenceLine) -> Threshold {
    let tone = match line.kind {
        ReferenceKind::Hard => Tone::Critical,
        ReferenceKind::Soft => Tone::Muted,
    };
    Threshold::new(line.value, line.label.clone()).tone(tone)
}

/// The accessible name of a chart: its heading and the range it covers.
fn label_of(def: &ChartDef, range_label: &str) -> String {
    format!("{}, last {range_label}", def.title)
}

/// Brings the name of a kept chart up to date with the range now shown.
pub(super) fn relabel(
    chart: &Entity<Chart>,
    def: &ChartDef,
    range_label: &str,
    cx: &mut Context<DetailView>,
) {
    let label = label_of(def, range_label);
    chart.update(cx, |chart, cx| chart.set_label(label, cx));
}

/// The chart entity for `data`, named for the screen reader by its heading and the range.
pub(super) fn new_chart(
    def: &ChartDef,
    range_label: &str,
    cx: &mut Context<DetailView>,
) -> Entity<Chart> {
    let label = label_of(def, range_label);
    let format = format_of(def.unit);
    let id = def.id;
    cx.new(move |_| Chart::line(id).format(format).label(label))
}

/// Puts `data` into `chart`: series, axis and reference lines. The chart redraws only when
/// something changed.
///
/// The time labels are in the machine's time zone, or in UTC when the view's clock is pinned
/// (screenshots and tests must not change with the zone of the machine they run on).
pub(super) fn show(
    chart: &Entity<Chart>,
    data: &ChartData,
    utc: bool,
    cx: &mut Context<DetailView>,
) {
    let series: Vec<Series> = data.series.iter().map(series_of).collect();
    let axis = if utc {
        TimeAxis::utc(data.from, data.to)
    } else {
        TimeAxis::local(data.from, data.to)
    };
    let thresholds: Vec<Threshold> = data.references.iter().map(threshold_of).collect();
    chart.update(cx, |chart, cx| {
        chart.set_unavailable(None, cx);
        chart.set_thresholds(thresholds, cx);
        chart.set_series(series, axis, cx);
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use oxikube_app::metrics_history::Sample;

    use super::*;

    #[test]
    fn units_pick_the_axis_format() {
        assert_eq!(format_of(MetricUnit::Cores), ValueFormat::Cores);
        assert_eq!(format_of(MetricUnit::Bytes), ValueFormat::Bytes);
        assert_eq!(
            format_of(MetricUnit::BytesPerSecond),
            ValueFormat::BytesPerSecond
        );
        assert_eq!(format_of(MetricUnit::Count), ValueFormat::Number);
    }

    #[test]
    fn a_missing_reading_is_a_gap_not_a_zero() {
        let data = SeriesData {
            name: "app".into(),
            samples: Arc::from(vec![
                Sample { t: 1.0, v: 0.5 },
                Sample {
                    t: 2.0,
                    v: f64::NAN,
                },
                Sample { t: 3.0, v: 0.7 },
            ]),
        };
        let series = series_of(&data);
        assert_eq!(series.points.len(), 3);
        assert!(series.points[1].is_gap());
        assert_eq!(series.points[2].v, 0.7);
    }

    #[test]
    fn limits_are_critical_and_requests_are_muted() {
        let limit = ReferenceLine {
            label: "limits".into(),
            value: 2.0,
            kind: ReferenceKind::Hard,
        };
        let request = ReferenceLine {
            label: "requests".into(),
            value: 1.0,
            kind: ReferenceKind::Soft,
        };
        assert_eq!(threshold_of(&limit).tone, Tone::Critical);
        assert_eq!(threshold_of(&request).tone, Tone::Muted);
        assert_eq!(threshold_of(&request).value, 1.0);
    }
}
