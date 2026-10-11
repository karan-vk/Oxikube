//! The sparkline's cached geometry: what it was built from, the projected points and the
//! tessellated paths, kept in element state between frames.

use super::data::{Point, SeriesColor};
use super::downsample::{Decimated, decimate};
use super::paint::paint_dot;
use super::palette::ChartPalette;
use super::paths;
use crate::size::current_scale;
use gpui::{Bounds, ContentMask, Pixels, Window, point, px, size};
use gpui_component::plot::{Grid, PathCache};
use std::sync::Arc;

/// Design-pixel inset so the stroke is not clipped at the edges.
const INSET: f32 = 2.0;

/// What the cached geometry was built from.
#[derive(Clone, PartialEq)]
pub(super) struct Key {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) scale: u32,
    pub(super) time: Option<(u64, u64)>,
    pub(super) value: Option<(u64, u64)>,
    pub(super) fill: bool,
    pub(super) marker: bool,
    pub(super) palette: ChartPalette,
    pub(super) color: SeriesColor,
}

/// Geometry and tessellated paths, kept in element state between frames.
#[derive(Default)]
pub(super) struct SparkState {
    source: Option<Arc<[Point]>>,
    key: Option<Key>,
    build: u64,
    points: Vec<gpui::Point<Pixels>>,
    runs: Vec<std::ops::Range<usize>>,
    base: Pixels,
    /// The latest sample's position, for the marker.
    last: Option<gpui::Point<Pixels>>,
    /// The dashed line standing for "no data".
    empty: Option<Grid>,
    stroke: PathCache,
    fill: PathCache,
}

pub(super) struct Inputs<'a> {
    pub(super) points: &'a Arc<[Point]>,
    pub(super) key: Key,
    pub(super) time: Option<(f64, f64)>,
    pub(super) value: Option<(f64, f64)>,
    pub(super) color: gpui::Hsla,
    pub(super) width: Pixels,
    pub(super) fill: bool,
    pub(super) marker: bool,
}

#[cfg(test)]
thread_local! {
    /// How many times a sparkline rebuilt its geometry on this thread (tests assert caching).
    pub(super) static REBUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl SparkState {
    pub(super) fn rebuild(
        &mut self,
        inputs: &Inputs,
        bounds: Bounds<Pixels>,
        palette: &ChartPalette,
    ) {
        #[cfg(test)]
        REBUILDS.with(|n| n.set(n.get() + 1));
        let inset = INSET * current_scale();
        let (w, h) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
        let (left, top) = (inset, inset);
        let (pw, ph) = ((w - 2.0 * inset).max(1.0), (h - 2.0 * inset).max(1.0));

        self.points.clear();
        self.runs.clear();
        self.last = None;
        self.empty = None;
        self.base = px(top + ph);

        let readings = inputs.points.iter().filter(|p| !p.is_gap());
        let (mut t0, mut t1) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut lo, mut hi) = (0.0f64, f64::NEG_INFINITY);
        for p in readings {
            t0 = t0.min(p.t);
            t1 = t1.max(p.t);
            lo = lo.min(p.v);
            hi = hi.max(p.v);
        }
        if !t0.is_finite() {
            // No readings: a dashed baseline says "nothing here" without a number.
            self.empty = Some(
                Grid::new()
                    .y([px(ph / 2.0)])
                    .stroke(palette.muted.opacity(0.6))
                    .dash_array(&[px(3.0 * current_scale()), px(3.0 * current_scale())]),
            );
            return;
        }
        let (t0, t1) = inputs.time.unwrap_or((t0, t1));
        let single = t1 <= t0;
        let (lo, hi) = inputs.value.unwrap_or((lo, hi));
        let hi = if hi > lo { hi } else { lo + 1.0 };
        let x = |t: f64| {
            if single {
                left + pw / 2.0
            } else {
                left + ((t - t0) / (t1 - t0)) as f32 * pw
            }
        };
        let y = |v: f64| {
            let ratio = ((v - lo) / (hi - lo)) as f32;
            (top + ph - ratio * ph).clamp(top - ph, top + 2.0 * ph)
        };

        let mut reduced = Decimated::default();
        let columns = ((pw / 2.0).round() as usize).clamp(1, 2_000);
        let (from, to) = if single {
            (t0 - 1.0, t0 + 1.0)
        } else {
            (t0, t1)
        };
        decimate(inputs.points, from, to, columns, &mut reduced);
        self.points
            .extend(reduced.points.iter().map(|p| point(px(x(p.t)), px(y(p.v)))));
        self.runs = reduced.runs;
        if inputs.marker {
            self.last = self.points.last().copied();
        }
    }
}

pub(super) fn paint_state(
    state: &mut SparkState,
    inputs: &Inputs,
    bounds: Bounds<Pixels>,
    palette: &ChartPalette,
    window: &mut Window,
) {
    let same = state
        .source
        .as_ref()
        .is_some_and(|s| Arc::ptr_eq(s, inputs.points))
        && state.key.as_ref() == Some(&inputs.key);
    if !same {
        state.rebuild(inputs, bounds, palette);
        state.source = Some(inputs.points.clone());
        state.key = Some(inputs.key.clone());
        state.build += 1;
    }
    let mask = ContentMask {
        bounds: Bounds::new(
            point(bounds.origin.x, bounds.origin.y - px(1.)),
            size(bounds.size.width, bounds.size.height + px(2.)),
        ),
    };
    window.with_content_mask(Some(mask), |window| {
        if let Some(empty) = &state.empty {
            empty.paint(&bounds, window);
            return;
        }
        let build = state.build;
        let SparkState {
            points,
            runs,
            base,
            stroke,
            fill,
            last,
            ..
        } = state;
        if inputs.fill
            && let Some(path) = fill.get(build, bounds.origin, || paths::fill(points, runs, *base))
        {
            window.paint_path(path, inputs.color.opacity(palette.area_alpha));
        }
        if let Some(path) = stroke.get(build, bounds.origin, || {
            paths::stroke(points, runs, inputs.width)
        }) {
            window.paint_path(path, inputs.color);
        }
        for dot in paths::lone_points(points, runs) {
            paint_dot(window, bounds.origin + dot, inputs.width, inputs.color);
        }
        if let Some(at) = last {
            paint_dot(
                window,
                bounds.origin + *at,
                inputs.width * 1.5,
                inputs.color,
            );
        }
    });
}
