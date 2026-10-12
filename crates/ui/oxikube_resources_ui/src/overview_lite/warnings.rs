//! The recent-warnings panel: the newest warning events of the cluster, from the
//! [`RecentWarnings`](oxikube_app::RecentWarnings) hook, or an empty state.
//!
//! The overview never lists Events itself (that is the Events feature's, E14): it asks the hook
//! for a few rows about once a second, on the same timer that reads the counts, and redraws only
//! when the rows changed.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div, px,
};
use oxikube_app::WarningRow;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, u};

use super::view::WorkloadsOverview;

/// How many warnings the panel shows.
pub const WARNING_ROWS: usize = 8;

/// The line a warning is drawn as: `Pod/web-0 · BackOff ×3`.
pub fn headline(row: &WarningRow) -> String {
    let mut line = match &row.namespace {
        Some(namespace) => format!("{namespace}/{} · {}", row.object, row.reason),
        None => format!("{} · {}", row.object, row.reason),
    };
    if row.count > 1 {
        line.push_str(&format!(" ×{}", row.count));
    }
    line
}

impl WorkloadsOverview {
    /// The panel under the tiles.
    pub(super) fn warnings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = if self.warnings.is_empty() {
            div()
                .debug_selector(|| "overview-warnings-empty".to_owned())
                .text_size(u(tokens.font.body))
                .text_color(colors.text_muted)
                .child("No recent warnings.")
                .into_any_element()
        } else {
            v_flex()
                .gap(u(tokens.spacing.sm))
                .children(self.warnings.iter().enumerate().map(|(ix, row)| {
                    v_flex()
                        .debug_selector(move || format!("overview-warning-{ix}"))
                        .gap(u(px(1.)))
                        .child(
                            h_flex()
                                .gap(u(tokens.spacing.sm))
                                .child(
                                    div()
                                        .size(u(px(8.)))
                                        .flex_none()
                                        .rounded_full()
                                        .bg(colors.warning),
                                )
                                .child(
                                    div()
                                        .text_size(u(tokens.font.body))
                                        .text_color(colors.text)
                                        .truncate()
                                        .child(headline(row)),
                                ),
                        )
                        .child(
                            div()
                                .pl(u(px(16.)))
                                .text_size(u(tokens.font.small))
                                .text_color(colors.text_muted)
                                .truncate()
                                .child(row.message.clone()),
                        )
                }))
                .into_any_element()
        };
        v_flex()
            .debug_selector(|| "overview-warnings".to_owned())
            .gap(u(tokens.spacing.md))
            .p(u(tokens.spacing.lg))
            .rounded(u(tokens.radius.md))
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.surface)
            .child(
                div()
                    .text_size(u(tokens.font.body))
                    .font_semibold()
                    .child("Recent warnings"),
            )
            .child(body)
            .into_any_element()
    }
}
