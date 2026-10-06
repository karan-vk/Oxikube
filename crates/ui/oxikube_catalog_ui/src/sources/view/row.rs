//! One row of the sources list.

use gpui::{
    Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Styled as _,
    div, px,
};
use oxikube_app::sources::SourceRow;
use oxikube_ports::{SourceState, UserSourceKind};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::SourcesView;
use crate::sources::model::status_text;

/// The height of every row, at 100 % zoom: uniform, as `uniform_list` needs. Tall enough for the
/// path and, under it, the status or the error.
pub(super) const ROW_HEIGHT: Pixels = px(52.);

fn kind_icon(row: &SourceRow) -> IconName {
    match row.source.kind {
        UserSourceKind::Default => IconName::Boxes,
        UserSourceKind::File => IconName::FileCode,
        UserSourceKind::Dir => IconName::Folder,
    }
}

impl SourcesView {
    pub(super) fn render_row(&mut self, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Some(row) = self.model.row(ix) else {
            return div().into_any_element();
        };
        let status = status_text(row);
        let is_error = row.is_error();
        let colour = match row.state {
            _ if is_error => colors.error,
            Some(SourceState::Blank) => colors.warning,
            // A note on a source that works (a folder with one bad file) is a warning.
            Some(SourceState::Found) if row.message.is_some() => colors.warning,
            _ => colors.text_muted,
        };
        let stored = row.stored;
        let label = row.label.clone();
        let icon = kind_icon(row);
        let busy = self.busy;

        let mut title = h_flex().gap(u(tokens.spacing.md)).items_center().child(
            div()
                .truncate()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(label),
        );
        if stored {
            title = title.child(
                div()
                    .flex_none()
                    .px(u(tokens.spacing.sm))
                    .rounded(u(tokens.radius.sm))
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .bg(colors.element_hover)
                    .child("stored by Oxikube"),
            );
        }
        let state_line: Div = div()
            .truncate()
            .text_size(u(tokens.font.small))
            .text_color(colour)
            .debug_selector(move || {
                format!(
                    "sources-status-{ix}{}",
                    if is_error { ":error" } else { "" }
                )
            })
            .child(status);

        h_flex()
            .id(("sources-row", ix))
            .debug_selector(move || format!("sources-row-{ix}"))
            .h(u(ROW_HEIGHT))
            .w_full()
            .flex_none()
            .gap(u(tokens.spacing.lg))
            .px(u(tokens.spacing.xl))
            .items_center()
            .border_b_1()
            .border_color(colors.border_variant)
            .child(Icon::new(icon).size(u(px(16.))).color(if is_error {
                colors.error
            } else {
                colors.text_muted
            }))
            .child(v_flex().flex_1().min_w_0().child(title).child(state_line))
            .child(
                div()
                    .debug_selector(move || format!("sources-remove-{ix}"))
                    .flex_none()
                    .child(
                        Button::new(("sources-remove", ix))
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::Trash).size(u(px(14.))))
                            .tooltip("Remove source")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_remove(ix, window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
