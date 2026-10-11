//! Decimation: at most a few points per pixel column, whatever the series length.
//!
//! A 24 h range at a 15 s step is 5 760 samples across perhaps 600 pixels. Drawing all of them
//! costs tessellation time for detail no pixel can show, so the geometry step reduces each series
//! to its **first, minimum, maximum and last** sample of every column (the M4 scheme): the line
//! keeps every spike and dip a full-resolution draw would show. Gaps split the series into runs
//! that are decimated separately, so a gap is never bridged.

use super::data::Point;
use std::ops::Range;

/// A series reduced for drawing: the kept samples, and which stretches of them are unbroken.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decimated {
    /// The kept samples (all finite), ascending in time.
    pub points: Vec<Point>,
    /// Index ranges into [`Decimated::points`], one per unbroken run (a gap ends a run).
    pub runs: Vec<Range<usize>>,
}

/// Reduces `points` to the window `start..=end` (plus the sample on each side, so a line runs off
/// the edge instead of stopping short) at `columns` pixel columns.
///
/// `columns` is the plot's width in pixels; at most `4 * columns` samples come out per run, and a
/// run with no more samples than that is passed through untouched.
pub fn decimate(points: &[Point], start: f64, end: f64, columns: usize, out: &mut Decimated) {
    out.points.clear();
    out.runs.clear();
    let columns = columns.max(1);
    let span = end - start;
    if points.is_empty() || span.is_nan() || span <= 0.0 {
        return;
    }
    // The visible window with one sample of margin on each side.
    let lo = points.partition_point(|p| p.t < start).saturating_sub(1);
    let hi = (points.partition_point(|p| p.t <= end) + 1).min(points.len());
    let window = &points[lo..hi];

    let mut run_start = 0;
    while run_start < window.len() {
        if window[run_start].is_gap() {
            run_start += 1;
            continue;
        }
        let mut run_end = run_start;
        while run_end < window.len() && !window[run_end].is_gap() {
            run_end += 1;
        }
        let first = out.points.len();
        reduce_run(
            &window[run_start..run_end],
            start,
            span,
            columns,
            &mut out.points,
        );
        out.runs.push(first..out.points.len());
        run_start = run_end;
    }
}

/// M4 over one unbroken run.
fn reduce_run(run: &[Point], start: f64, span: f64, columns: usize, out: &mut Vec<Point>) {
    if run.len() <= columns * 4 {
        out.extend_from_slice(run);
        return;
    }
    let scale = columns as f64 / span;
    let column = |t: f64| ((t - start) * scale).floor().clamp(-1.0, columns as f64) as i64;

    let mut i = 0;
    while i < run.len() {
        let col = column(run[i].t);
        let mut j = i + 1;
        while j < run.len() && column(run[j].t) == col {
            j += 1;
        }
        // The bucket is run[i..j]: keep first, min, max, last in time order.
        let (mut lo, mut hi) = (i, i);
        for k in i..j {
            if run[k].v < run[lo].v {
                lo = k;
            }
            if run[k].v > run[hi].v {
                hi = k;
            }
        }
        let mut keep = [i, lo, hi, j - 1];
        keep.sort_unstable();
        let mut previous = usize::MAX;
        for k in keep {
            if k != previous {
                out.push(run[k]);
                previous = k;
            }
        }
        i = j;
    }
}

/// One value per pixel column for bars: the largest sample of each column, so a bar chart of a
/// long series still shows its peaks. Gaps and samples outside `start..=end` are dropped.
pub fn bucket_max(points: &[Point], start: f64, end: f64, columns: usize, out: &mut Vec<Point>) {
    out.clear();
    let columns = columns.max(1);
    let span = end - start;
    if span.is_nan() || span <= 0.0 {
        return;
    }
    let scale = columns as f64 / span;
    let mut current: Option<(i64, Point)> = None;
    for p in points
        .iter()
        .filter(|p| !p.is_gap() && p.t >= start && p.t <= end)
    {
        let col = (((p.t - start) * scale).floor() as i64).min(columns as i64 - 1);
        match &mut current {
            Some((c, best)) if *c == col => {
                if p.v > best.v {
                    *best = *p;
                }
            }
            _ => {
                if let Some((_, best)) = current.take() {
                    out.push(best);
                }
                current = Some((col, *p));
            }
        }
    }
    if let Some((_, best)) = current {
        out.push(best);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Vec<Point> {
        (0..n).map(|i| Point::new(i as f64, i as f64)).collect()
    }

    fn run(points: &[Point], start: f64, end: f64, columns: usize) -> Decimated {
        let mut out = Decimated::default();
        decimate(points, start, end, columns, &mut out);
        out
    }

    #[test]
    fn short_series_pass_through() {
        let pts = ramp(50);
        let d = run(&pts, 0.0, 49.0, 100);
        assert_eq!(d.points, pts);
        assert_eq!(d.runs, [0..50]);
    }

    #[test]
    fn long_series_are_bounded_by_the_columns() {
        let pts = ramp(5_760);
        let d = run(&pts, 0.0, 5_759.0, 600);
        assert!(d.points.len() <= 4 * 600, "{}", d.points.len());
        assert!(d.points.len() > 600);
        assert_eq!(d.points.first(), pts.first());
        assert_eq!(d.points.last(), pts.last());
        assert!(d.points.windows(2).all(|w| w[0].t < w[1].t));
    }

    #[test]
    fn a_single_spike_survives() {
        let mut pts = ramp(5_760);
        for p in &mut pts {
            p.v = 1.0;
        }
        pts[3_001].v = 100.0;
        pts[4_500].v = -50.0;
        let d = run(&pts, 0.0, 5_759.0, 300);
        assert!(d.points.iter().any(|p| p.v == 100.0), "spike lost");
        assert!(d.points.iter().any(|p| p.v == -50.0), "dip lost");
    }

    #[test]
    fn gaps_split_runs_and_are_never_bridged() {
        let mut pts = ramp(10);
        pts[4] = Point::gap(4.0);
        pts[5] = Point::gap(5.0);
        let d = run(&pts, 0.0, 9.0, 100);
        assert_eq!(d.runs, [0..4, 4..8]);
        assert!(d.points.iter().all(|p| !p.is_gap()));
        assert!(d.points.iter().all(|p| p.t != 4.0 && p.t != 5.0));
    }

    #[test]
    fn all_gaps_and_empty_series_draw_nothing() {
        let gaps = vec![Point::gap(1.0), Point::gap(2.0)];
        assert!(run(&gaps, 0.0, 10.0, 100).points.is_empty());
        assert!(run(&[], 0.0, 10.0, 100).runs.is_empty());
        assert!(run(&ramp(5), 5.0, 5.0, 100).points.is_empty());
    }

    #[test]
    fn only_the_visible_window_is_kept_plus_a_sample_either_side() {
        let pts = ramp(100);
        let d = run(&pts, 40.0, 60.0, 1_000);
        assert_eq!(d.points.first().map(|p| p.t), Some(39.0));
        assert_eq!(d.points.last().map(|p| p.t), Some(61.0));
    }

    #[test]
    fn a_single_sample_is_one_run_of_one() {
        let d = run(&[Point::new(5.0, 2.0)], 0.0, 10.0, 100);
        assert_eq!(d.runs, [0..1]);
    }

    #[test]
    fn reusing_the_buffer_does_not_grow_it() {
        let pts = ramp(5_760);
        let mut out = Decimated::default();
        decimate(&pts, 0.0, 5_759.0, 600, &mut out);
        let capacity = out.points.capacity();
        decimate(&pts, 0.0, 5_759.0, 600, &mut out);
        assert_eq!(out.points.capacity(), capacity);
    }

    #[test]
    fn bars_keep_the_peak_of_each_column() {
        let mut pts = ramp(1_000);
        pts[500].v = 9_999.0;
        let mut out = Vec::new();
        bucket_max(&pts, 0.0, 999.0, 100, &mut out);
        assert!(out.len() <= 100);
        assert!(out.iter().any(|p| p.v == 9_999.0));
        assert!(out.windows(2).all(|w| w[0].t < w[1].t));
    }
}
