//! The hover tooltip of a [`Chart`]: what it lists, and the crosshair, dots and box drawn for it.

use super::hover;
use super::palette::ChartPalette;
use super::ticks::tooltip_time;
use super::view::Chart;
use crate::u;
use gpui::{AnyElement, IntoElement as _, SharedString, point, px};
use gpui_component::plot::tooltip::{CrossLine, Dot, Tooltip};

/// How far (in pixels) another series' sample may be from the hovered one and still be listed.
const ALIGN_PX: f64 = 6.0;

/// One line of the hover tooltip.
#[derive(Clone, Debug, PartialEq)]
pub struct HoverRow {
    /// The series name.
    pub name: SharedString,
    /// The series colour (the swatch).
    pub color: gpui::Hsla,
    /// The value, written in the chart's unit.
    pub value: String,
    /// Where the series' dot sits, relative to the chart's canvas.
    pub at: gpui::Point<gpui::Pixels>,
}

impl Chart {
    /// What the tooltip lists for the hovered sample: each visible series' colour, name, the value
    /// in the chart's unit, and where its dot sits (canvas-relative).
    pub fn hover_rows(&self, palette: &ChartPalette) -> Vec<HoverRow> {
        let (Some(t), cache) = (self.hovered, self.cache.borrow()) else {
            return Vec::new();
        };
        let Some(layout) = cache.layout.as_ref().filter(|l| l.drawable) else {
            return Vec::new();
        };
        let tolerance = ALIGN_PX * layout.seconds_per_pixel();
        hover::readings(&self.spec, t, tolerance)
            .into_iter()
            .map(|(index, reading)| {
                let series = &self.spec.series[index];
                HoverRow {
                    name: series.name.clone(),
                    color: palette.resolve(series.style.color, index),
                    value: self.spec.format.format(reading.v),
                    at: point(px(layout.x_of(reading.t)), px(layout.y_of(reading.v))),
                }
            })
            .collect()
    }

    /// The crosshair, dots and tooltip for the hovered sample.
    pub(super) fn overlay(&self, palette: &ChartPalette) -> Option<AnyElement> {
        let t = self.hovered?;
        let rows = self.hover_rows(palette);
        let first = rows.first()?;
        let cache = self.cache.borrow();
        let plot = cache.layout.as_ref()?.plot;
        let x = first.at.x;
        let top = rows
            .iter()
            .map(|r| r.at.y)
            .fold(first.at.y, |a, b| a.min(b));
        // The box hugs the highest dot, with a gap so it clears it.
        let mut overlay = Tooltip::new(point(x, top), cache.canvas_size)
            .glide(false)
            .gap(u(px(12.)))
            .title(tooltip_time(t, &self.spec.axis))
            .cross_line(
                CrossLine::new(point(x, px(0.)))
                    .span(plot.origin.y.as_f32(), plot.size.height.as_f32()),
            )
            .dots(rows.iter().map(|row| {
                Dot::new(row.at)
                    .size(u(px(8.)))
                    .fill(row.color)
                    .stroke(palette.background)
            }));
        for row in rows {
            overlay = overlay.row(row.color, row.name, row.value);
        }
        Some(overlay.into_any_element())
    }
}
