//! Painting a cached layout.

use super::cache::LayoutCache;
use super::palette::ChartPalette;
use super::paths;
use super::spec::{ChartKind, Spec};
use gpui::{App, Bounds, ContentMask, Hsla, Pixels, Point, Window, fill, point, px, quad, size};

/// Paints the chart in `bounds` (the canvas): bands, grid, series, thresholds, labels.
pub(super) fn paint(
    cache: &mut LayoutCache,
    generation: u64,
    spec: &Spec,
    palette: &ChartPalette,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    cache.origin = bounds.origin;
    cache.canvas_size = bounds.size;
    cache.ensure(generation, spec, bounds, palette, window);
    let LayoutCache {
        layout,
        strokes,
        fills,
        build,
        ..
    } = cache;
    let Some(layout) = layout.as_ref().filter(|l| l.drawable) else {
        return;
    };
    let plot = Bounds::new(bounds.origin + layout.plot.origin, layout.plot.size);

    for band in &layout.bands {
        let at = Bounds::new(bounds.origin + band.bounds.origin, band.bounds.size);
        window.paint_quad(fill(at, band.color));
    }
    layout.grid.paint(&plot, window);
    layout.baseline.paint(&plot, window);

    // Lines run off the plot's edges (the sample either side of the window); clip them to it,
    // with room for the stroke and the single-sample dots.
    let room = px(4.);
    let clip = Bounds::new(
        point(plot.origin.x - room, plot.origin.y - room),
        size(plot.size.width + room * 2., plot.size.height + room * 2.),
    );
    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
        for (slot, series) in layout.series.iter().enumerate() {
            match spec.kind {
                ChartKind::Bar => {
                    for bar in &series.bars {
                        let at = Bounds::new(bounds.origin + bar.origin, bar.size);
                        window.paint_quad(fill(at, series.color));
                    }
                }
                ChartKind::Line | ChartKind::Area => {
                    if spec.kind == ChartKind::Area
                        && let Some(path) = fills[slot].get(*build, bounds.origin, || {
                            paths::fill(&series.points, &series.runs, layout.fill_base)
                        })
                    {
                        window.paint_path(path, series.color.opacity(palette.area_alpha));
                    }
                    if let Some(path) = strokes[slot].get(*build, bounds.origin, || {
                        paths::stroke(&series.points, &series.runs, series.width)
                    }) {
                        window.paint_path(path, series.color);
                    }
                    for dot in paths::lone_points(&series.points, &series.runs) {
                        paint_dot(window, bounds.origin + dot, series.width, series.color);
                    }
                }
            }
        }
    });

    for threshold in &layout.thresholds {
        threshold.grid.paint(&plot, window);
    }
    layout.labels.paint(&bounds, window, cx);
}

/// A filled circle of about twice the stroke width: how a lone sample shows.
pub(super) fn paint_dot(window: &mut Window, at: Point<Pixels>, stroke: Pixels, color: Hsla) {
    let radius = (stroke * 1.4).max(px(2.));
    window.paint_quad(quad(
        Bounds::new(
            point(at.x - radius, at.y - radius),
            size(radius * 2., radius * 2.),
        ),
        radius,
        color,
        px(0.),
        gpui::transparent_black(),
        gpui::BorderStyle::default(),
    ));
}
