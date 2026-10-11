//! Hover lookup: which sample is under the cursor, and what each series reads there.
//!
//! Pure functions over the samples (binary search, no allocation per series), so the chart can
//! decide on every mouse move whether anything changed and only then notify.

use super::data::Point;
use super::spec::Spec;

/// The finite sample of `points` nearest to time `t`, looking only at the two samples that
/// bracket it (a hover never snaps across a gap to a far-away sample).
pub(super) fn nearest(points: &[Point], t: f64) -> Option<Point> {
    let at = points.partition_point(|p| p.t < t);
    let before = at.checked_sub(1).and_then(|i| points.get(i));
    let after = points.get(at);
    [before, after]
        .into_iter()
        .flatten()
        .filter(|p| !p.is_gap())
        .min_by(|a, b| (a.t - t).abs().total_cmp(&(b.t - t).abs()))
        .copied()
}

/// The time of the sample nearest to `t` among the visible series. This is the hover's identity:
/// the chart notifies only when it changes.
///
/// A series only answers inside its own time range, plus `edge` seconds either side (so the empty
/// stretch of an axis the data has not reached yet shows no tooltip).
pub(super) fn snap(spec: &Spec, t: f64, edge: f64) -> Option<f64> {
    spec.visible()
        .filter(|(_, s)| match (s.points.first(), s.points.last()) {
            (Some(first), Some(last)) => t >= first.t - edge && t <= last.t + edge,
            _ => false,
        })
        .filter_map(|(_, s)| nearest(&s.points, t))
        .map(|p| p.t)
        .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
}

/// What each visible series reads at the snapped time `t`: its own nearest sample if that is within
/// `tolerance` seconds of `t` (series are rarely sampled at exactly the same instant).
pub(super) fn readings(spec: &Spec, t: f64, tolerance: f64) -> Vec<(usize, Point)> {
    spec.visible()
        .filter_map(|(index, s)| {
            nearest(&s.points, t)
                .filter(|p| (p.t - t).abs() <= tolerance)
                .map(|p| (index, p))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{ChartKind, Series};

    fn pts(items: &[(f64, f64)]) -> Vec<Point> {
        items.iter().map(|(t, v)| Point::new(*t, *v)).collect()
    }

    #[test]
    fn nearest_picks_the_closer_neighbour() {
        let p = pts(&[(0.0, 1.0), (10.0, 2.0), (20.0, 3.0)]);
        assert_eq!(nearest(&p, 4.0).map(|p| p.t), Some(0.0));
        assert_eq!(nearest(&p, 6.0).map(|p| p.t), Some(10.0));
        assert_eq!(nearest(&p, -50.0).map(|p| p.t), Some(0.0));
        assert_eq!(nearest(&p, 99.0).map(|p| p.t), Some(20.0));
        assert_eq!(nearest(&[], 1.0), None);
    }

    #[test]
    fn nearest_skips_gaps_without_crossing_them() {
        let p = vec![
            Point::new(0.0, 1.0),
            Point::gap(10.0),
            Point::gap(20.0),
            Point::new(30.0, 2.0),
        ];
        // Between a reading and a gap: the reading, even when the gap is closer.
        assert_eq!(nearest(&p, 8.0).map(|p| p.t), Some(0.0));
        // Inside a long gap neither bracketing sample is a reading: nothing.
        assert_eq!(nearest(&p, 15.0), None);
    }

    #[test]
    fn snap_keys_on_the_sample_time_not_the_cursor() {
        let mut spec = Spec::new(ChartKind::Line);
        spec.series = vec![Series::new(
            "a",
            pts(&[(0.0, 1.0), (10.0, 2.0), (20.0, 3.0)]),
        )];
        let a = snap(&spec, 11.0, 5.0);
        let b = snap(&spec, 13.0, 5.0);
        assert_eq!(a, Some(10.0));
        assert_eq!(
            a, b,
            "moving within one sample's reach must not change the hover"
        );
        assert_eq!(snap(&spec, 16.0, 5.0), Some(20.0));
        assert_eq!(snap(&spec, 500.0, 5.0), None);
    }

    #[test]
    fn readings_follow_each_series_and_respect_hidden() {
        let mut spec = Spec::new(ChartKind::Line);
        spec.series = vec![
            Series::new("a", pts(&[(0.0, 1.0), (10.0, 2.0)])),
            Series::new("b", pts(&[(1.0, 5.0), (11.0, 6.0)])),
            Series::new("c", pts(&[(100.0, 9.0)])),
        ];
        let rows = readings(&spec, 10.0, 2.0);
        let got: Vec<(usize, f64)> = rows.iter().map(|(i, p)| (*i, p.v)).collect();
        assert_eq!(got, [(0, 2.0), (1, 6.0)], "c has nothing near t=10");
        spec.hidden = vec!["a".into()];
        let got: Vec<usize> = readings(&spec, 10.0, 2.0).iter().map(|(i, _)| *i).collect();
        assert_eq!(got, [1]);
    }
}
