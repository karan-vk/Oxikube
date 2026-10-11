//! Drawing helpers the kinds' own panels share: a titled block, a muted note, a fixed-width cell.

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px,
};
use oxikube_ui::layout::{StyledExt as _, v_flex};
use oxikube_ui::{Tokens, u};

/// A titled block: `PORTS (2)`.
pub(in crate::kinds) fn section(tokens: &Tokens, title: &'static str, count: usize) -> gpui::Div {
    let id = title;
    v_flex()
        .debug_selector(move || format!("service-section-{id}"))
        .w_full()
        .child(
            div()
                .px(u(tokens.spacing.lg))
                .pt(u(tokens.spacing.md))
                .pb(u(px(2.)))
                .text_size(u(tokens.font.small))
                .font_semibold()
                .text_color(tokens.colors.text_muted)
                .child(format!("{} ({count})", title.to_uppercase())),
        )
}

pub(in crate::kinds) fn note(tokens: &Tokens, text: &str) -> AnyElement {
    div()
        .px(u(tokens.spacing.lg))
        .py(u(px(2.)))
        .whitespace_normal()
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .child(text.to_owned())
        .into_any_element()
}

/// One fixed-width cell of a row, cut with an ellipsis.
pub(in crate::kinds) fn cell(
    text: impl Into<SharedString>,
    width: f32,
    color: gpui::Hsla,
) -> impl IntoElement {
    div()
        .flex_none()
        .w(u(px(width)))
        .truncate()
        .text_color(color)
        .child(text.into())
}
