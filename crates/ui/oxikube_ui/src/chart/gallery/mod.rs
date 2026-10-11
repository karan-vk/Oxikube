//! A gallery of every chart state on one page: data, several series, thresholds, a long series,
//! gaps, a single sample, "No data" with and without a reason, and sparklines.
//!
//! Reviewers use it to see the components side by side (it is what `tests/chart_screenshot.rs`
//! photographs, and the app opens it from the command palette as "Chart gallery"), and the perf
//! scenario `charts` draws the same fixtures. The data are deterministic functions of `now`, so a
//! screenshot is stable.

pub mod fixtures;

use super::{
    Band, Chart, Point, Series, SeriesColor, Sparkline, Threshold, TimeAxis, Tone, ValueFormat,
    YRange,
};
use crate::layout::{h_flex, v_flex};
use crate::tokens::ActiveTokens as _;
use crate::u;
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use std::sync::Arc;

/// Height of every chart card's plot.
const CARD_HEIGHT: f32 = 168.0;

/// The page. Create with [`ChartGallery::new`].
pub struct ChartGallery {
    cards: Vec<(SharedString, Entity<Chart>)>,
    sparks: Vec<(SharedString, Arc<[Point]>, SeriesColor, bool)>,
    spark_axis: (f64, f64),
}

impl ChartGallery {
    /// Builds every chart over data that ends at `now` (seconds since the epoch).
    pub fn new(now: f64, cx: &mut Context<Self>) -> Self {
        let hour = TimeAxis::utc(now - 3600.0, now);
        let day = TimeAxis::utc(now - 86_400.0, now);
        let mut cards = Vec::new();
        let mut add = |title: &str, chart: Chart, series: Vec<Series>, axis: TimeAxis| {
            let entity = cx.new(|_| chart);
            entity.update(cx, |chart, cx| chart.set_series(series, axis, cx));
            cards.push((SharedString::from(title.to_owned()), entity));
        };

        add(
            "Line, three containers, request and limit",
            Chart::line("gallery-line")
                .label("CPU per container")
                .format(ValueFormat::Cores)
                .thresholds(vec![
                    Threshold::new(0.5, "request"),
                    Threshold::new(0.9, "limit").tone(Tone::Critical),
                ])
                .bands(vec![
                    Band::new(0.7, 0.9, Tone::Warn),
                    Band::new(0.9, 1.0, Tone::Critical),
                ])
                .y_range(YRange::Fixed(0.0, 1.0)),
            vec![
                fixtures::wave("nginx", now, 240, 15.0, 0.45, 0.2, 40.0, 1),
                fixtures::wave("sidecar", now, 240, 15.0, 0.12, 0.05, 25.0, 2),
                fixtures::wave("init-db", now, 240, 15.0, 0.62, 0.25, 60.0, 3),
            ],
            hour,
        );
        add(
            "Area, memory over 24 hours (5 760 samples)",
            Chart::area("gallery-area")
                .label("Memory")
                .format(ValueFormat::Bytes)
                .thresholds(vec![Threshold::new(3.0 * GIB, "limit").tone(Tone::Warn)]),
            vec![fixtures::wave(
                "working set",
                now,
                5_760,
                15.0,
                1.6 * GIB,
                0.5 * GIB,
                900.0,
                4,
            )],
            day,
        );
        add(
            "Bar, network by pod",
            Chart::bar("gallery-bar")
                .label("Network")
                .format(ValueFormat::BytesPerSecond),
            vec![
                fixtures::wave("rx", now, 72, 300.0, 4.0e6, 3.0e6, 18.0, 5),
                fixtures::wave("tx", now, 72, 300.0, 1.5e6, 1.0e6, 12.0, 6),
            ],
            TimeAxis::utc(now - 6.0 * 3600.0, now),
        );
        add(
            "Gaps (scrape failures) are not drawn as zero",
            Chart::line("gallery-gaps")
                .label("Gaps")
                .format(ValueFormat::Percent),
            vec![fixtures::with_gaps(
                fixtures::wave("usage", now, 240, 15.0, 55.0, 25.0, 50.0, 7),
                37,
            )],
            hour,
        );
        add(
            "A single sample",
            Chart::line("gallery-single").label("Single").legend(false),
            vec![Series::new("only", vec![Point::new(now - 600.0, 3.0)])],
            hour,
        );
        let none = cx.new(|_| Chart::line("gallery-none").label("No endpoint"));
        none.update(cx, |chart, cx| {
            chart.set_unavailable(Some("No Prometheus endpoint".into()), cx)
        });
        cards.push(("No data, with the reason".into(), none));
        let empty = cx.new(|_| Chart::area("gallery-empty").label("Empty"));
        empty.update(cx, |chart, cx| {
            chart.set_series(vec![Series::new("cpu", Vec::<Point>::new())], hour, cx);
        });
        cards.push(("No data, no reason".into(), empty));

        let spark = |name: &str, color, fill, seed| {
            let s = fixtures::wave(name, now, 120, 15.0, 40.0, 25.0, 30.0, seed);
            (SharedString::from(name.to_owned()), s.points, color, fill)
        };
        let sparks = vec![
            spark("categorical", SeriesColor::Auto, false, 11),
            spark("filled", SeriesColor::Auto, true, 12),
            spark("good", SeriesColor::Tone(Tone::Good), true, 13),
            spark("warn", SeriesColor::Tone(Tone::Warn), false, 14),
            spark("critical", SeriesColor::Tone(Tone::Critical), true, 15),
            (
                "no data".into(),
                Arc::from(Vec::<Point>::new()),
                SeriesColor::Auto,
                false,
            ),
            (
                "one sample".into(),
                Arc::from(vec![Point::new(now, 5.0)]),
                SeriesColor::Auto,
                false,
            ),
        ];
        Self {
            cards,
            sparks,
            spark_axis: (now - 1800.0, now),
        }
    }

    /// The chart entities, in page order (tests drive them).
    pub fn charts(&self) -> impl Iterator<Item = &Entity<Chart>> {
        self.cards.iter().map(|(_, chart)| chart)
    }
}

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

impl Render for ChartGallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let card = |title: SharedString, body: gpui::AnyElement| {
            v_flex()
                .gap(u(tokens.spacing.md))
                .p(u(tokens.spacing.lg))
                .rounded(u(tokens.radius.md))
                .border_1()
                .border_color(colors.border_variant)
                .bg(colors.surface)
                .child(
                    div()
                        .text_size(u(tokens.font.small))
                        .text_color(colors.text_muted)
                        .child(title),
                )
                .child(body)
        };
        let sparks = self
            .sparks
            .iter()
            .enumerate()
            .map(|(i, (name, points, color, fill))| {
                v_flex()
                    .gap(u(tokens.spacing.xs))
                    .w(u(px(132.)))
                    .child(
                        Sparkline::new(("gallery-spark", i), points.clone())
                            .time_range(self.spark_axis.0, self.spark_axis.1)
                            .color(*color)
                            .filled(*fill)
                            .last_marker(true)
                            .label(name.clone())
                            .height(px(28.)),
                    )
                    .child(
                        div()
                            .text_size(u(tokens.font.small))
                            .text_color(colors.text_muted)
                            .child(name.clone()),
                    )
            });
        v_flex()
            .id("chart-gallery")
            .size_full()
            .overflow_y_scroll()
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xl))
            .bg(colors.background)
            .text_color(colors.text)
            .children(self.cards.iter().map(|(title, chart)| {
                card(
                    title.clone(),
                    div()
                        .h(u(px(CARD_HEIGHT)))
                        .w_full()
                        .child(chart.clone())
                        .into_any_element(),
                )
            }))
            .child(card(
                "Sparklines".into(),
                h_flex()
                    .flex_wrap()
                    .gap(u(tokens.spacing.xl))
                    .children(sparks)
                    .into_any_element(),
            ))
    }
}
