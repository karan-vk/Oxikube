//! Tessellation: projected points to GPUI paths. Called when the geometry changes, never per
//! frame (the results live in [`gpui_component::plot::PathCache`]s).

use gpui::{Path, PathBuilder, Pixels, Point};
use std::ops::Range;

/// The stroke through every run, as one path (a gap is a new sub-path). `None` when there is
/// nothing to stroke, or the tessellator refused the input.
pub(super) fn stroke(
    points: &[Point<Pixels>],
    runs: &[Range<usize>],
    width: Pixels,
) -> Option<Path<Pixels>> {
    let mut builder = PathBuilder::stroke(width);
    let mut any = false;
    for run in runs.iter().filter(|r| r.len() >= 2) {
        let run = &points[run.clone()];
        builder.move_to(run[0]);
        for p in &run[1..] {
            builder.line_to(*p);
        }
        any = true;
    }
    if any { builder.build().ok() } else { None }
}

/// The fill between every run and the horizontal line at `base`, one closed polygon per run.
pub(super) fn fill(
    points: &[Point<Pixels>],
    runs: &[Range<usize>],
    base: Pixels,
) -> Option<Path<Pixels>> {
    let mut builder = PathBuilder::fill();
    let mut any = false;
    for run in runs.iter().filter(|r| r.len() >= 2) {
        let run = &points[run.clone()];
        builder.move_to(run[0]);
        for p in &run[1..] {
            builder.line_to(*p);
        }
        builder.line_to(Point {
            x: run[run.len() - 1].x,
            y: base,
        });
        builder.line_to(Point {
            x: run[0].x,
            y: base,
        });
        builder.close();
        any = true;
    }
    if any { builder.build().ok() } else { None }
}

/// The points of runs that are a single sample: nothing to stroke, so they are drawn as dots.
pub(super) fn lone_points<'a>(
    points: &'a [Point<Pixels>],
    runs: &'a [Range<usize>],
) -> impl Iterator<Item = Point<Pixels>> + 'a {
    runs.iter()
        .filter(|r| r.len() == 1)
        .map(|r| points[r.start])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};

    fn line(n: usize) -> Vec<Point<Pixels>> {
        (0..n)
            .map(|i| point(px(i as f32), px((i % 5) as f32)))
            .collect()
    }

    #[test]
    fn a_run_of_two_or_more_strokes() {
        let pts = line(10);
        assert!(stroke(&pts, &[0..10], px(1.5)).is_some());
        assert!(fill(&pts, &[0..10], px(20.)).is_some());
    }

    #[test]
    fn single_samples_have_no_path_but_are_reported_as_dots() {
        let pts = line(3);
        assert!(stroke(&pts, &[0..1, 2..3], px(1.5)).is_none());
        assert!(fill(&pts, &[1..2], px(20.)).is_none());
        let dots: Vec<_> = lone_points(&pts, &[0..1, 1..3, 2..3]).collect();
        assert_eq!(dots.len(), 2);
    }

    #[test]
    fn gaps_make_separate_sub_paths() {
        let pts = line(10);
        let together = stroke(&pts, &[0..10], px(1.5)).map(|p| p.vertices.len());
        let split = stroke(&pts, &[0..4, 6..10], px(1.5)).map(|p| p.vertices.len());
        assert!(split < together, "{split:?} !< {together:?}");
    }

    #[test]
    fn five_thousand_points_fit_the_index_buffer() {
        // GPUI tessellates into u16 indices; the decimator keeps a plot under 4 x 2000 samples.
        let pts = line(8_000);
        assert!(stroke(&pts, &[0..8_000], px(1.5)).is_some());
        assert!(fill(&pts, &[0..8_000], px(20.)).is_some());
    }
}
