//! Drawing the Metrics tab: the range selector, then a virtualised list of chart cards, or the
//! state that stands in for them (loading skeleton, empty, error with Retry).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, list, px,
};
use oxikube_domain::RangePreset;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::state::MetricsState;
use crate::detail::parts::{empty, full_width, skeleton};
use crate::detail::view::DetailView;
use crate::table::ToneColors;

/// Height of one chart card, unscaled pixels: heading, plot and legend.
const CARD_HEIGHT: f32 = 210.;

impl DetailView {
    /// The Metrics tab. The range selector is always there, so a click shows the loading state
    /// in the same frame whatever the tab was showing.
    pub(in crate::detail) fn metrics_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.metrics_state();
        let content = match &state {
            MetricsState::NoEndpoint(reason) => self.metrics_note(
                "metrics-unavailable",
                format!("metrics unavailable: {reason}"),
                false,
                cx,
            ),
            MetricsState::Loading => skeleton(
                &cx.tokens(),
                "metrics-loading",
                &[220., 320., 180., 300., 240.],
            ),
            MetricsState::Empty => {
                let subject_pods_missing = self
                    .metrics_subject()
                    .is_some_and(|subject| subject.is_empty());
                let text = if subject_pods_missing {
                    "This workload has no pods to graph.".to_owned()
                } else {
                    format!(
                        "Prometheus has no series for this object in the last {}. A new object, \
                         or one the Prometheus does not scrape, shows nothing yet.",
                        self.metrics.range.label()
                    )
                };
                self.metrics_note("metrics-empty", text, false, cx)
            }
            MetricsState::Error { message, retryable } => {
                self.metrics_note("metrics-error", message.clone(), *retryable, cx)
            }
            MetricsState::Ready => div()
                .debug_selector(|| "metrics-charts".to_owned())
                .size_full()
                .child(
                    list(
                        self.metrics.list.clone(),
                        cx.processor(|this, ix, _, cx| full_width(this.metric_card(ix, cx))),
                    )
                    .size_full(),
                )
                .into_any_element(),
        };
        v_flex()
            .debug_selector(|| "detail-metrics".to_owned())
            .size_full()
            .child(self.metrics_toolbar(cx))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }

    /// The segmented range control, and a caption on the right.
    fn metrics_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let current = self.metrics.range;
        let caption = match self.metrics.pods_left_out {
            0 => String::new(),
            left => format!("{left} more pods not shown"),
        };
        h_flex()
            .debug_selector(|| "metrics-toolbar".to_owned())
            .flex_none()
            .items_center()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.md))
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .child(
                h_flex()
                    .gap(u(px(2.)))
                    .children(RangePreset::ALL.iter().enumerate().map(|(i, &preset)| {
                        let label = preset.label();
                        div()
                            .debug_selector(move || format!("metrics-range-{label}"))
                            .child(
                                Button::new(("metrics-range", i))
                                    .xsmall()
                                    .label(label)
                                    .when(preset == current, |b| b.primary())
                                    .when(preset != current, |b| b.ghost())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.set_metrics_range(preset, cx)
                                    })),
                            )
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_right()
                    .text_size(u(tokens.font.small))
                    .text_color(tokens.colors.text_muted)
                    .child(caption),
            )
            .into_any_element()
    }

    /// A message in place of the charts, with Retry for an error that may pass.
    fn metrics_note(
        &self,
        selector: &'static str,
        text: String,
        retry: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let error = selector == "metrics-error";
        v_flex()
            .debug_selector(move || selector.to_owned())
            .gap(u(tokens.spacing.md))
            .items_start()
            .p(u(tokens.spacing.xl))
            .child(
                div()
                    .whitespace_normal()
                    .text_color(if error {
                        tones.error
                    } else {
                        tokens.colors.text_muted
                    })
                    .child(text),
            )
            .when(retry, |col| {
                col.child(
                    div().debug_selector(|| "metrics-retry".to_owned()).child(
                        Button::new("metrics-retry")
                            .xsmall()
                            .label("Retry")
                            .on_click(cx.listener(|this, _, _, cx| this.retry_metrics(cx))),
                    ),
                )
            })
            .into_any_element()
    }

    /// One chart card. Only cards on screen are built.
    fn metric_card(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(slot) = self.metrics.charts.get(ix) else {
            return empty();
        };
        let tokens = cx.tokens();
        let key = slot.key;
        let title = slot.title.clone();
        v_flex()
            .debug_selector(move || format!("metrics-chart-{key}"))
            .h(u(px(CARD_HEIGHT)))
            .px(u(tokens.spacing.lg))
            .pt(u(tokens.spacing.md))
            .pb(u(tokens.spacing.sm))
            .gap(u(tokens.spacing.xs))
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .child(
                div()
                    .flex_none()
                    .text_size(u(tokens.font.small))
                    .font_semibold()
                    .text_color(tokens.colors.text_muted)
                    .child(title),
            )
            .child(div().flex_1().min_h_0().child(slot.chart.clone()))
            .into_any_element()
    }
}
