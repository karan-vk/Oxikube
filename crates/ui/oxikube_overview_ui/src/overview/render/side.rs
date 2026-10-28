//! The side column: the cluster-info card and the recent warnings feed.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px,
};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, u};

use super::{card, muted};
use crate::overview::view::ClusterOverview;

impl ClusterOverview {
    pub(super) fn info_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let info = self.info();
        let rows = [
            ("overview-info-context", "Context", info.context),
            ("overview-info-version", "Version", info.version),
            (
                "overview-info-distribution",
                "Distribution",
                info.distribution,
            ),
            ("overview-info-nodes", "Nodes", info.nodes),
            ("overview-info-server", "API server", info.server),
        ]
        .into_iter()
        .map(|(id, label, value)| {
            h_flex()
                .gap(u(tokens.spacing.md))
                .text_size(u(tokens.font.small))
                .child(
                    div()
                        .w(u(px(88.)))
                        .flex_none()
                        .text_color(tokens.colors.text_muted)
                        .child(label),
                )
                .child(
                    div()
                        .debug_selector(move || id.to_owned())
                        .min_w_0()
                        .truncate()
                        .child(SharedString::from(value)),
                )
        });
        card("overview-info", "Cluster", cx)
            .children(rows)
            .into_any_element()
    }

    pub(super) fn warnings_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let mut card = card("overview-warnings", "Recent warnings", cx);
        if self.warnings.is_empty() {
            return card
                .child(muted("No recent warnings", "overview-warnings-empty", cx))
                .into_any_element();
        }
        for (ix, warning) in self.warnings.iter().enumerate() {
            let object = match &warning.namespace {
                Some(ns) => format!("{ns}/{}", warning.object),
                None => warning.object.clone(),
            };
            let count = (warning.count > 1).then(|| format!(" ×{}", warning.count));
            card = card.child(
                v_flex()
                    .debug_selector(move || format!("overview-warning-{ix}"))
                    .gap(u(px(2.)))
                    .text_size(u(tokens.font.small))
                    .child(
                        h_flex()
                            .gap(u(tokens.spacing.sm))
                            .child(
                                div()
                                    .text_color(tokens.colors.warning)
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(SharedString::from(warning.reason.clone())),
                            )
                            .child(div().min_w_0().truncate().child(SharedString::from(format!(
                                "{object}{}",
                                count.unwrap_or_default()
                            )))),
                    )
                    .child(
                        div()
                            .text_color(tokens.colors.text_muted)
                            .line_clamp(2)
                            .child(SharedString::from(warning.message.clone())),
                    ),
            );
        }
        card.into_any_element()
    }
}
