//! [`PulseTile`]: one tile of the page, built per frame from the model.
//!
//! Fixed size, whatever the numbers say: the title row, the value, the caption, the gauge's slot
//! and the sparkline's slot always take the same height, and an unselected tile has a border of
//! the same width as a selected one, so a count that gains a digit or a selection that moves
//! shifts nothing.

use std::sync::Arc;

use gpui::{
    App, ClickEvent, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use oxikube_ui::chart::{Point, SeriesColor, Sparkline, Tone};
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, Icon, u};

use super::face::{Level, TileFace};
use super::gauge::Gauge;

/// Design pixels of a tile's height.
pub const TILE_HEIGHT: f32 = 140.0;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A tile: title and status, value, caption, gauge and sparkline.
#[derive(IntoElement)]
pub struct PulseTile {
    index: usize,
    id: &'static str,
    title: &'static str,
    face: TileFace,
    points: Arc<[Point]>,
    selected: bool,
    on_click: Option<ClickHandler>,
}

impl PulseTile {
    /// The tile at `index` of the page, `id` its stable name (the selector, the sparkline's id).
    pub fn new(
        index: usize,
        id: &'static str,
        title: &'static str,
        face: TileFace,
        points: Arc<[Point]>,
    ) -> Self {
        Self {
            index,
            id,
            title,
            face,
            points,
            selected: false,
            on_click: None,
        }
    }

    /// Whether the tile has the keyboard selection (a focus ring).
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Runs `handler` on a click.
    #[must_use]
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for PulseTile {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let value_colour = match self.face.level {
            Level::Neutral => colors.text,
            Level::Good => colors.success,
            Level::Warn => colors.warning,
            Level::Critical => colors.error,
            Level::Muted => colors.text_muted,
        };
        let spark = match self.face.level {
            Level::Good => SeriesColor::Tone(Tone::Good),
            Level::Warn => SeriesColor::Tone(Tone::Warn),
            Level::Critical => SeriesColor::Tone(Tone::Critical),
            Level::Neutral => SeriesColor::Tone(Tone::Info),
            Level::Muted => SeriesColor::Tone(Tone::Muted),
        };
        let selector = format!("pulse-tile-{}", self.id);
        let status = self.face.level.status();
        let icon = self.face.level.icon();
        let (gauge_fraction, gauge_text) = match &self.face.gauge {
            Some(gauge) => (Some(gauge.fraction), gauge.text.clone()),
            None => (None, String::new()),
        };
        let hover = self.face.hover.clone();
        let border = if self.selected {
            colors.border_focused
        } else {
            gpui::transparent_black()
        };
        let mut tile = v_flex()
            .id(ElementId::from(("pulse-tile", self.index)))
            .debug_selector(move || selector)
            .flex_1()
            .min_w(u(px(0.)))
            .h(u(px(TILE_HEIGHT)))
            .gap(u(tokens.spacing.xs))
            .p(u(tokens.spacing.md))
            .rounded(u(tokens.radius.md))
            .border_2()
            .border_color(border)
            .bg(colors.surface)
            .child(
                h_flex()
                    .h(u(px(16.)))
                    .gap(u(tokens.spacing.sm))
                    .justify_between()
                    .child(
                        div()
                            .min_w(u(px(0.)))
                            .text_size(u(tokens.font.small))
                            .text_color(colors.text_muted)
                            .truncate()
                            .child(self.title),
                    )
                    .when_some(status.zip(icon), |row, (status, icon)| {
                        row.child(
                            h_flex()
                                .flex_none()
                                .gap(u(tokens.spacing.xs))
                                .text_size(u(tokens.font.small))
                                .text_color(value_colour)
                                .child(Icon::new(icon).size(u(px(12.))))
                                .child(status),
                        )
                    }),
            )
            .child(
                div()
                    .h(u(px(32.)))
                    .text_size(u(px(26.)))
                    .text_color(value_colour)
                    .font_semibold()
                    .truncate()
                    .child(self.face.value),
            )
            .child(
                div()
                    .h(u(px(16.)))
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .truncate()
                    .child(self.face.caption),
            )
            .child(div().py(u(px(2.))).child(Gauge::new(
                ElementId::from(("pulse-gauge", self.index)),
                gauge_fraction,
                self.face.level,
                SharedString::from(gauge_text),
            )))
            .child(
                Sparkline::new(ElementId::from(("pulse-spark", self.index)), self.points)
                    .color(spark)
                    .filled(true)
                    .last_marker(true)
                    .height(px(28.))
                    .label(SharedString::from(format!("{} trend", self.title))),
            );
        if let Some(handler) = self.on_click {
            tile = tile
                .cursor_pointer()
                .hover(move |style| style.bg(colors.element_hover))
                .on_click(move |event, window, cx| handler(event, window, cx));
        }
        match hover {
            Some(text) => tile
                .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
                .into_any_element(),
            None => tile.into_any_element(),
        }
    }
}
