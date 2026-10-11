//! The "no data" state: an explicit message, with the reason when there is one.

use crate::layout::v_flex;
use crate::tokens::ActiveTokens as _;
use crate::u;
use gpui::{
    App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};

/// What a chart shows in place of a plot when there is nothing to plot: "No data", and under it
/// why ("No Prometheus endpoint", "metrics unavailable: RBAC denied").
///
/// It fills its parent and centres its text, so it is the same size as the chart it replaces and
/// the layout does not jump when data arrives.
#[derive(IntoElement)]
pub struct NoData {
    reason: Option<SharedString>,
}

impl NoData {
    /// A "No data" placeholder, with the reason underneath when given.
    pub fn new(reason: Option<SharedString>) -> Self {
        Self { reason }
    }
}

impl RenderOnce for NoData {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(u(tokens.spacing.xs))
            .px(u(tokens.spacing.lg))
            .text_color(colors.text_muted)
            .child(
                div()
                    .text_size(u(tokens.font.body))
                    .child(SharedString::from("No data")),
            )
            .when_some(self.reason, |this, reason| {
                this.child(
                    div()
                        .text_size(u(tokens.font.small))
                        .max_w_full()
                        .text_center()
                        .child(reason),
                )
            })
    }
}
