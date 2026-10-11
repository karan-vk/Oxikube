//! [`Sparkline`]: a compact trend line with no axes, for table cells, tiles and pulses.

use super::data::{Point, SeriesColor};
use super::palette::ChartPalette;
use super::spark_state::{Inputs, Key, SparkState, paint_state};
use crate::size::current_scale;
use crate::tokens::ActiveTokens as _;
use crate::u;
use gpui::{
    App, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, canvas,
    div, prelude::FluentBuilder as _, px,
};
use std::sync::Arc;

/// The stroke width in design pixels.
const STROKE: f32 = 1.25;

/// A trend line without axes or tooltip, fixed height, filling its width.
///
/// Built per frame like any element, but cheap: the points are decimated to the pixel columns and
/// tessellated once, then kept in element state until the `Arc` of samples is replaced (pass the
/// same `Arc` every frame while the data has not changed), the size, zoom or theme changes.
///
/// `id` must be unique among the sparklines drawn from the same parent (for example the row's
/// object name plus the column).
#[derive(IntoElement)]
pub struct Sparkline {
    id: ElementId,
    points: Arc<[Point]>,
    time: Option<(f64, f64)>,
    value: Option<(f64, f64)>,
    color: SeriesColor,
    fill: bool,
    marker: bool,
    height: Pixels,
    width: Option<Pixels>,
    label: Option<SharedString>,
}

impl Sparkline {
    /// A sparkline of `points` (ascending in time).
    pub fn new(id: impl Into<ElementId>, points: Arc<[Point]>) -> Self {
        Self {
            id: id.into(),
            points,
            time: None,
            value: None,
            color: SeriesColor::Auto,
            fill: false,
            marker: false,
            height: px(18.),
            width: None,
            label: None,
        }
    }

    /// The time range to span (default: the first to the last sample). Pass the same range to
    /// every sparkline of a column to line them up.
    #[must_use]
    pub fn time_range(mut self, start: f64, end: f64) -> Self {
        self.time = Some((start, end));
        self
    }

    /// The value range to span (default: zero, or the minimum when negative, up to the maximum).
    /// Pass `0.0..=limit` to show utilisation against a limit.
    #[must_use]
    pub fn value_range(mut self, min: f64, max: f64) -> Self {
        self.value = Some((min, max));
        self
    }

    /// The line colour (default: the first categorical colour).
    #[must_use]
    pub fn color(mut self, color: SeriesColor) -> Self {
        self.color = color;
        self
    }

    /// Fills the area under the line.
    #[must_use]
    pub fn filled(mut self, fill: bool) -> Self {
        self.fill = fill;
        self
    }

    /// Marks the latest sample with a dot.
    #[must_use]
    pub fn last_marker(mut self, marker: bool) -> Self {
        self.marker = marker;
        self
    }

    /// The height in design pixels (default 18; zoom is applied).
    #[must_use]
    pub fn height(mut self, height: Pixels) -> Self {
        self.height = height;
        self
    }

    /// A fixed width in design pixels instead of filling the parent.
    #[must_use]
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
    }

    /// The accessible name ("CPU trend").
    #[must_use]
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl RenderOnce for Sparkline {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let palette = ChartPalette::from_tokens(&tokens);
        let state: Entity<SparkState> =
            window.use_keyed_state(self.id.clone(), cx, |_, _| SparkState::default());
        let label = self.label.clone().unwrap_or_else(|| "Trend".into());
        let latest = self.points.iter().rev().find(|p| !p.is_gap()).map(|p| p.v);
        let summary: SharedString = match latest {
            Some(v) => format!("{label}: latest {v:.3}").into(),
            None => format!("{label}: no data").into(),
        };
        let color = palette.resolve(self.color, 0);
        let Sparkline {
            id,
            points,
            time,
            value,
            fill,
            marker,
            height,
            width,
            color: source,
            ..
        } = self;
        let bits = |(a, b): (f64, f64)| (a.to_bits(), b.to_bits());
        div()
            .id(id)
            .role(Role::Image)
            .aria_label(summary)
            .h(u(height))
            .flex_none()
            .map(|el| match width {
                Some(width) => el.w(u(width)),
                None => el.w_full(),
            })
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, cx| {
                        let key = Key {
                            width: bounds.size.width.as_f32().to_bits(),
                            height: bounds.size.height.as_f32().to_bits(),
                            scale: current_scale().to_bits(),
                            time: time.map(bits),
                            value: value.map(bits),
                            fill,
                            marker,
                            palette,
                            color: source,
                        };
                        let inputs = Inputs {
                            points: &points,
                            key,
                            time,
                            value,
                            color,
                            width: u(px(STROKE)),
                            fill,
                            marker,
                        };
                        state.update(cx, |state, _| {
                            paint_state(state, &inputs, bounds, &palette, window);
                        });
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
#[path = "sparkline_tests.rs"]
mod tests;
