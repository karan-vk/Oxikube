//! The capacity and allocation card: per resource a bar of what the pods request (the fill),
//! where their limits reach (a mark) and what they use now (a thin bar under it), against what
//! the nodes can allocate; with the usage trend as a sparkline when Prometheus answers.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, div, px, relative,
};
use oxikube_domain::AllocationResource;
use oxikube_ui::chart::{Point, SeriesColor, Sparkline};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Tokens, u};

use super::{card, muted};
use crate::overview::model::{CapacityLine, live_metrics_loading};
use crate::overview::view::ClusterOverview;

/// Requests past this share of allocatable turn the fill amber, past [`FULL`] red.
const BUSY: f32 = 0.7;
/// See [`BUSY`].
const FULL: f32 = 0.9;

fn fill_colour(line: &CapacityLine, tokens: &Tokens) -> Hsla {
    let fill = line.requests_fill();
    if line.overcommitted() || fill >= FULL {
        tokens.colors.error
    } else if fill >= BUSY {
        tokens.colors.warning
    } else {
        tokens.colors.success
    }
}

fn id_of(resource: AllocationResource) -> &'static str {
    match resource {
        AllocationResource::Cpu => "cpu",
        AllocationResource::Memory => "memory",
        AllocationResource::Pods => "pods",
    }
}

/// One resource's bar: a track, the requests fill, the limits mark, the usage bar under it.
fn bar(line: &CapacityLine, tokens: &Tokens) -> AnyElement {
    let id = id_of(line.resource);
    let track = div()
        .debug_selector(move || format!("overview-capacity-{id}-bar"))
        .relative()
        .h(u(px(10.)))
        .w_full()
        .rounded(u(tokens.radius.sm))
        .bg(tokens.colors.element)
        .overflow_hidden()
        .child(
            div()
                .debug_selector(move || format!("overview-capacity-{id}-requests"))
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(line.requests_fill()))
                .bg(fill_colour(line, tokens)),
        )
        .children(
            line.limits_fill()
                .filter(|_| line.allocatable.is_some())
                .map(|at| {
                    div()
                        .absolute()
                        .top_0()
                        .h_full()
                        .w(u(px(2.)))
                        .left(relative((at - 0.01).max(0.0)))
                        .bg(tokens.colors.text_muted)
                }),
        );
    let usage = line.usage_fill().map(|fill| {
        div()
            .debug_selector(move || format!("overview-capacity-{id}-usage"))
            .h(u(px(4.)))
            .w_full()
            .rounded(u(tokens.radius.sm))
            .bg(tokens.colors.element)
            .overflow_hidden()
            .child(div().h_full().w(relative(fill)).bg(tokens.colors.info))
    });
    v_flex()
        .gap(u(px(3.)))
        .w_full()
        .child(track)
        .children(usage)
        .into_any_element()
}

fn spark(id: &'static str, points: &Arc<[Point]>, range: (f64, f64)) -> AnyElement {
    Sparkline::new(id, points.clone())
        .time_range(range.0, range.1)
        .color(SeriesColor::Auto)
        .filled(true)
        .height(px(28.))
        .width(px(120.))
        .label("Usage over the last hour")
        .into_any_element()
}

impl ClusterOverview {
    pub(super) fn capacity(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let sparks = self.history.sparks();
        let rows = self.shown.lines.iter().map(|line| {
            let id = id_of(line.resource);
            let trend = sparks.as_ref().and_then(|s| match line.resource {
                AllocationResource::Cpu => Some(spark("overview-spark-cpu", &s.cpu, s.range)),
                AllocationResource::Memory => {
                    Some(spark("overview-spark-memory", &s.memory, s.range))
                }
                AllocationResource::Pods => None,
            });
            h_flex()
                .gap(u(tokens.spacing.lg))
                .items_center()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(u(tokens.spacing.xs))
                        .child(
                            h_flex()
                                .gap(u(tokens.spacing.md))
                                .child(
                                    div()
                                        .w(u(px(64.)))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .child(line.resource.title()),
                                )
                                .child(
                                    div()
                                        .debug_selector(move || format!("overview-capacity-{id}"))
                                        .min_w_0()
                                        .truncate()
                                        .text_size(u(tokens.font.small))
                                        .text_color(tokens.colors.text_muted)
                                        .child(SharedString::from(line.summary())),
                                ),
                        )
                        .child(bar(line, &tokens)),
                )
                .children(trend)
        });
        let mut card = card("overview-capacity", "Capacity and allocation", cx).children(rows);
        card = card.child(legend(&tokens));
        if let Some(text) = self.shown.notice.clone() {
            card = card.child(self.notice("overview-live-notice", text, cx));
        } else if live_metrics_loading(&self.shown.snapshot) && self.deps.metrics.is_some() {
            card = card.child(muted(
                "Reading usage from metrics-server…",
                "overview-live-loading",
                cx,
            ));
        }
        card.into_any_element()
    }
}

fn legend(tokens: &Tokens) -> AnyElement {
    let swatch = |colour: Hsla, label: &'static str| {
        h_flex()
            .gap(u(px(4.)))
            .items_center()
            .child(div().size(u(px(8.))).rounded(u(px(2.))).bg(colour))
            .child(label)
    };
    h_flex()
        .gap(u(tokens.spacing.lg))
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .child(swatch(tokens.colors.success, "requested"))
        .child(swatch(tokens.colors.text_muted, "limits"))
        .child(swatch(tokens.colors.info, "used"))
        .into_any_element()
}
