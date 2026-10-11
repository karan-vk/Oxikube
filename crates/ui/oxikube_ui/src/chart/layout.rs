//! Geometry: scales, decimated and projected series, tick labels. Built once per data, size or
//! theme change and painted from every frame after (see [`super::cache`]).

use super::data::{Point, YRange};
use super::downsample::{Decimated, bucket_max, decimate};
use super::palette::ChartPalette;
use super::spec::{ChartKind, Spec};
use super::ticks::{time_ticks, value_labels, value_ticks};
use crate::size::{current_scale, u};
use gpui::{Bounds, Hsla, Pixels, SharedString, Size, TextAlign, Window, point, px, size};
use gpui_component::plot::{
    Grid, PlotLabel,
    label::{Text, measure_text_width},
};
use std::ops::Range;

/// Axis label size in design pixels.
const AXIS_FONT: f32 = 11.0;
/// The widest the plot is decimated for: more columns than this is a screen no one has.
const MAX_COLUMNS: usize = 2_000;

/// A shape's colour and where its points landed. Coordinates are relative to the canvas origin.
pub(super) struct SeriesGeometry {
    pub color: Hsla,
    pub width: Pixels,
    /// Projected, decimated samples (lines and areas).
    pub points: Vec<gpui::Point<Pixels>>,
    /// Unbroken stretches of `points`.
    pub runs: Vec<Range<usize>>,
    /// Bar rectangles (bar charts).
    pub bars: Vec<Bounds<Pixels>>,
}

/// A reference line.
pub(super) struct ThresholdGeometry {
    pub grid: Grid,
}

/// A shaded band, relative to the canvas.
pub(super) struct BandGeometry {
    pub bounds: Bounds<Pixels>,
    pub color: Hsla,
}

/// Everything painted for one chart at one size.
pub(super) struct Layout {
    /// The plot rectangle, relative to the canvas origin.
    pub plot: Bounds<Pixels>,
    t0: f64,
    t1: f64,
    y0: f64,
    y1: f64,
    pub grid: Grid,
    pub baseline: Grid,
    pub thresholds: Vec<ThresholdGeometry>,
    pub bands: Vec<BandGeometry>,
    pub labels: PlotLabel,
    pub series: Vec<SeriesGeometry>,
    /// Baseline y of an area fill, relative to the canvas.
    pub fill_base: Pixels,
    /// Whether the canvas was large enough to lay a plot out.
    pub drawable: bool,
}

impl Layout {
    /// A layout with nothing to paint (the canvas is too small).
    fn empty(size: Size<Pixels>) -> Self {
        Self {
            plot: Bounds::new(point(px(0.), px(0.)), size),
            t0: 0.0,
            t1: 1.0,
            y0: 0.0,
            y1: 1.0,
            grid: Grid::new(),
            baseline: Grid::new(),
            thresholds: Vec::new(),
            bands: Vec::new(),
            labels: PlotLabel::default(),
            series: Vec::new(),
            fill_base: px(0.),
            drawable: false,
        }
    }

    /// X (relative to the canvas) of time `t`.
    pub fn x_of(&self, t: f64) -> f32 {
        let ratio = ((t - self.t0) / (self.t1 - self.t0)) as f32;
        self.plot.origin.x.as_f32() + ratio * self.plot.size.width.as_f32()
    }

    /// Y (relative to the canvas) of value `v`, kept within a plot height of the plot so an
    /// outlier cannot send a path to coordinates the rasteriser would mangle.
    pub fn y_of(&self, v: f64) -> f32 {
        let ratio = ((v - self.y0) / (self.y1 - self.y0)) as f32;
        let top = self.plot.origin.y.as_f32();
        let height = self.plot.size.height.as_f32();
        (top + height - ratio * height).clamp(top - height, top + 2.0 * height)
    }

    /// The time at canvas-relative `x`, or `None` outside the plot's horizontal extent.
    pub fn time_at(&self, x: f32) -> Option<f64> {
        let left = self.plot.origin.x.as_f32();
        let width = self.plot.size.width.as_f32();
        if width <= 0.0 || x < left || x > left + width {
            return None;
        }
        Some(self.t0 + f64::from((x - left) / width) * (self.t1 - self.t0))
    }

    /// Seconds per pixel column.
    pub fn seconds_per_pixel(&self) -> f64 {
        (self.t1 - self.t0) / f64::from(self.plot.size.width.as_f32().max(1.0))
    }

    /// Builds the layout of `spec` in a canvas of `size`.
    pub fn build(
        spec: &Spec,
        size: Size<Pixels>,
        palette: &ChartPalette,
        window: &mut Window,
    ) -> Self {
        let scale = current_scale();
        let font = u(px(AXIS_FONT));
        let gap = 6.0 * scale;

        // 1. The value axis: extent, nice ticks, labels.
        let (lo, hi) = value_extent(spec);
        let unit = spec.format.tick_unit(hi.abs().max(lo.abs()));
        let gutter_h = font.as_f32() + 2.0 * gap;
        let top = gap;
        let height_estimate = (size.height.as_f32() - top - gutter_h).max(1.0);
        let target = ((height_estimate / (40.0 * scale)).floor() as usize).clamp(2, 6);
        let y_ticks = value_ticks(lo, hi, target, unit);
        let (y0, y1) = y_ticks.domain();
        let y_texts = value_labels(&y_ticks, &spec.format);
        let widest = y_texts
            .iter()
            .map(|t| measure_text_width(&SharedString::from(t.clone()), font, window))
            .fold(0.0f32, f32::max);

        // 2. The plot rectangle.
        let sample = if spec.axis.span() <= 300.0 {
            "00:00:00"
        } else {
            "00:00"
        };
        let label_w = measure_text_width(&SharedString::from(sample), font, window);
        let left = widest + gap;
        let right = label_w / 2.0 + 4.0 * scale;
        let plot = Bounds::new(
            point(px(left), px(top)),
            gpui::size(
                px((size.width.as_f32() - left - right).max(0.0)),
                px((size.height.as_f32() - top - gutter_h).max(0.0)),
            ),
        );
        if plot.size.width < px(8.) || plot.size.height < px(8.) {
            return Self::empty(size);
        }
        let mut layout = Self {
            drawable: true,
            plot,
            t0: spec.axis.start,
            t1: spec.axis.end,
            y0,
            y1,
            ..Self::empty(size)
        };
        let (plot_w, plot_h) = (plot.size.width.as_f32(), plot.size.height.as_f32());
        let bottom = plot.origin.y.as_f32() + plot_h;
        layout.fill_base = px(layout.y_of(0.0f64.clamp(y0, y1)));

        // 3. Grid, baseline, labels.
        layout.grid = Grid::new()
            .y(y_ticks.values.iter().map(|v| px(layout.y_of(*v) - top)))
            .stroke(palette.grid);
        layout.baseline = Grid::new().y([px(plot_h)]).stroke(palette.baseline);
        let mut texts = Vec::new();
        for (value, text) in y_ticks.values.iter().zip(y_texts) {
            texts.push(
                Text::new(
                    text,
                    point(
                        px(left - gap),
                        px(layout.y_of(*value) - font.as_f32() / 2.0),
                    ),
                    palette.axis_text,
                )
                .font_size(font)
                .align(TextAlign::Right),
            );
        }
        for tick in time_ticks(&spec.axis, plot_w, label_w) {
            texts.push(
                Text::new(
                    tick.label,
                    point(px(layout.x_of(tick.t)), px(bottom + gap * 0.75)),
                    palette.axis_text,
                )
                .font_size(font)
                .align(TextAlign::Center),
            );
        }

        // 4. Bands and thresholds.
        for band in &spec.bands {
            let (from, to) = (band.from.max(y0), band.to.min(y1));
            if to > from {
                let (a, b) = (layout.y_of(to), layout.y_of(from));
                layout.bands.push(BandGeometry {
                    bounds: Bounds::new(
                        point(plot.origin.x, px(a)),
                        gpui::size(plot.size.width, px((b - a).max(1.0))),
                    ),
                    color: palette.tone(band.tone).opacity(palette.band_alpha),
                });
            }
        }
        for threshold in &spec.thresholds {
            if threshold.value < y0 || threshold.value > y1 {
                continue;
            }
            let y = layout.y_of(threshold.value);
            let color = palette.tone(threshold.tone);
            layout.thresholds.push(ThresholdGeometry {
                grid: Grid::new()
                    .y([px(y - top)])
                    .stroke(color)
                    .dash_array(&[px(4. * scale), px(3. * scale)]),
            });
            texts.push(
                Text::new(
                    threshold.label.clone(),
                    point(
                        px(left + plot_w - 4.0 * scale),
                        px(y - font.as_f32() - 2.0 * scale),
                    ),
                    color,
                )
                .font_size(font)
                .align(TextAlign::Right),
            );
        }
        layout.labels = PlotLabel::new(texts);

        // 5. Series.
        // One decimation column per two logical pixels: M4 keeps four samples per column, so a
        // line still has two points per pixel (every spike and dip intact) at half the tessellation.
        let columns = ((plot_w / 2.0).round() as usize).clamp(1, MAX_COLUMNS);
        match spec.kind {
            ChartKind::Bar => layout.add_bars(spec, palette, columns),
            ChartKind::Line | ChartKind::Area => layout.add_lines(spec, palette, columns),
        }
        layout
    }

    fn add_lines(&mut self, spec: &Spec, palette: &ChartPalette, columns: usize) {
        let mut reduced = Decimated::default();
        for (index, series) in spec.visible() {
            decimate(&series.points, self.t0, self.t1, columns, &mut reduced);
            self.series.push(SeriesGeometry {
                color: palette.resolve(series.style.color, index),
                width: u(px(series.style.width)),
                points: reduced
                    .points
                    .iter()
                    .map(|p| point(px(self.x_of(p.t)), px(self.y_of(p.v))))
                    .collect(),
                runs: std::mem::take(&mut reduced.runs),
                bars: Vec::new(),
            });
        }
    }

    fn add_bars(&mut self, spec: &Spec, palette: &ChartPalette, columns: usize) {
        let mut buckets: Vec<(usize, &super::Series, Vec<Point>)> = Vec::new();
        for (index, series) in spec.visible() {
            let mut out = Vec::new();
            bucket_max(&series.points, self.t0, self.t1, columns, &mut out);
            buckets.push((index, series, out));
        }
        // The slot of one bar group: the closest two samples of any series, in pixels.
        let per_pixel = self.seconds_per_pixel();
        let slot = buckets
            .iter()
            .flat_map(|(_, _, pts)| pts.windows(2).map(|w| w[1].t - w[0].t))
            .filter(|dt| *dt > 0.0)
            .fold(f64::INFINITY, f64::min);
        let slot_px = if slot.is_finite() {
            (slot / per_pixel) as f32
        } else {
            self.plot.size.width.as_f32()
        };
        let group = (slot_px * 0.8).clamp(1.0, 48.0 * current_scale());
        let count = buckets.len().max(1) as f32;
        let each = (group / count).max(1.0);
        let base = self.y_of(0.0f64.clamp(self.y0, self.y1));
        for (slot_index, (index, series, pts)) in buckets.into_iter().enumerate() {
            let bars = pts
                .iter()
                .map(|p| {
                    let x = self.x_of(p.t) - group / 2.0 + each * slot_index as f32;
                    let y = self.y_of(p.v);
                    let (top, bottom) = if y <= base { (y, base) } else { (base, y) };
                    Bounds::new(
                        point(px(x), px(top)),
                        size(px(each), px((bottom - top).max(1.0))),
                    )
                })
                .collect();
            self.series.push(SeriesGeometry {
                color: palette.resolve(series.style.color, index),
                width: u(px(series.style.width)),
                points: Vec::new(),
                runs: Vec::new(),
                bars,
            });
        }
    }
}

/// The value range to scale for: the visible series within the time axis, any threshold, and
/// (for [`YRange::Auto`]) zero.
fn value_extent(spec: &Spec) -> (f64, f64) {
    if let YRange::Fixed(lo, hi) = spec.y_range {
        return (lo, hi);
    }
    let (start, end) = (spec.axis.start, spec.axis.end);
    let (mut lo, mut hi) = (0.0f64, 0.0f64);
    for (_, series) in spec.visible() {
        let points = &series.points;
        let from = points.partition_point(|p| p.t < start).saturating_sub(1);
        let to = (points.partition_point(|p| p.t <= end) + 1).min(points.len());
        for p in points[from..to].iter().filter(|p| !p.is_gap()) {
            lo = lo.min(p.v);
            hi = hi.max(p.v);
        }
    }
    for threshold in &spec.thresholds {
        if threshold.value.is_finite() {
            lo = lo.min(threshold.value);
            hi = hi.max(threshold.value);
        }
    }
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{Series, Threshold};

    fn spec(values: &[f64]) -> Spec {
        let mut spec = Spec::new(ChartKind::Line);
        let points: Vec<Point> = values
            .iter()
            .enumerate()
            .map(|(i, v)| Point::new(i as f64, *v))
            .collect();
        spec.series = vec![Series::new("a", points)];
        spec.axis = crate::chart::TimeAxis::utc(0.0, values.len() as f64);
        spec.refresh();
        spec
    }

    #[test]
    fn auto_extent_includes_zero_and_thresholds() {
        assert_eq!(value_extent(&spec(&[3.0, 9.0, 5.0])), (0.0, 9.0));
        assert_eq!(value_extent(&spec(&[-4.0, 2.0])), (-4.0, 2.0));
        let mut with_limit = spec(&[1.0, 2.0]);
        with_limit.thresholds = vec![Threshold::new(8.0, "limit")];
        assert_eq!(value_extent(&with_limit), (0.0, 8.0));
    }

    #[test]
    fn nan_and_hidden_series_do_not_move_the_extent() {
        let mut s = spec(&[1.0, f64::NAN, 4.0]);
        assert_eq!(value_extent(&s), (0.0, 4.0));
        s.hidden = vec!["a".into()];
        assert_eq!(value_extent(&s), (0.0, 0.0));
    }

    #[test]
    fn fixed_ranges_are_used_as_given() {
        let mut s = spec(&[1.0, 500.0]);
        s.y_range = YRange::Fixed(0.0, 100.0);
        assert_eq!(value_extent(&s), (0.0, 100.0));
    }
}
