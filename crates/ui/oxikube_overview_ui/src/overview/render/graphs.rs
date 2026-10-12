//! The history graphs (Prometheus mode): CPU and memory over the last hour. Not drawn at all
//! when the app has no history metrics; a `metrics unavailable: <reason>` line with the way to
//! the settings when it has them but the cluster has no endpoint.

use gpui::{AnyElement, Context, IntoElement, ParentElement as _, Styled as _, div, px};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, u};

use super::{card, muted};
use crate::overview::history::HistoryState;
use crate::overview::view::ClusterOverview;

/// Height of a graph's plot.
const GRAPH_HEIGHT: f32 = 160.0;

impl ClusterOverview {
    pub(super) fn graphs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tokens = cx.tokens();
        let history = &self.history;
        let mut card = card("overview-history", "Last hour (Prometheus)", cx);
        match history.state() {
            HistoryState::Off => return None,
            HistoryState::Unavailable(_) => {
                let text = history.unavailable_text().unwrap_or_default();
                return Some(
                    card.child(self.notice("overview-history-notice", text, cx))
                        .into_any_element(),
                );
            }
            HistoryState::Detecting => {
                card = card.child(muted(
                    "Looking for a Prometheus-compatible service…",
                    "overview-history-detecting",
                    cx,
                ));
            }
            HistoryState::Failed(_) => {
                let text = history.unavailable_text().unwrap_or_default();
                card = card.child(self.notice("overview-history-notice", text, cx));
            }
            HistoryState::Loading | HistoryState::Ready => {}
        }
        let chart = |title: &'static str, entity: gpui::Entity<oxikube_ui::chart::Chart>| {
            gpui::div()
                .flex_1()
                .min_w(u(px(240.)))
                .child(
                    div()
                        .text_size(u(tokens.font.small))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(title),
                )
                .child(div().h(u(px(GRAPH_HEIGHT))).child(entity))
        };
        Some(
            card.child(
                h_flex()
                    .gap(u(tokens.spacing.lg))
                    .child(chart("CPU (cores)", history.cpu_chart().clone()))
                    .child(chart("Memory", history.memory_chart().clone())),
            )
            .into_any_element(),
        )
    }
}
