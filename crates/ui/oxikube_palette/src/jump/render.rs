//! The pieces of the bar's frame that are the jump bar's own: the problem line with the offending
//! word underlined, a completion's detail, and the key hints.

use gpui::{
    AnyElement, App, HighlightStyle, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, Styled as _, StyledText, UnderlineStyle, div, px,
};
use oxikube_app::search::jump::ParseError;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, u};

/// The line the user typed with the part `error` points at underlined, and what is wrong with it.
/// `error` is what Enter found (red, stays until the next key); `syntax` is what the parser says
/// of the line so far (a warning, quiet).
pub(super) fn header(
    line: &str,
    error: Option<&ParseError>,
    syntax: Option<&ParseError>,
    cx: &App,
) -> Option<AnyElement> {
    let tokens = cx.tokens();
    let (problem, color) = match (error, syntax) {
        (Some(error), _) => (error, tokens.colors.error),
        (None, Some(syntax)) => (syntax, tokens.colors.warning),
        (None, None) => return None,
    };
    let span = problem.span;
    let underline = HighlightStyle {
        color: Some(color),
        underline: Some(UnderlineStyle {
            thickness: px(1.),
            color: Some(color),
            wavy: true,
        }),
        ..HighlightStyle::default()
    };
    let echo = StyledText::new(SharedString::from(line.to_owned()));
    let echo = if span.is_empty()
        || !line.is_char_boundary(span.start)
        || !line.is_char_boundary(span.end)
        || span.end > line.len()
    {
        echo
    } else {
        echo.with_highlights([(span.range(), underline)])
    };
    Some(
        v_flex()
            .debug_selector(|| "jump-problem".into())
            .flex_none()
            .gap(u(tokens.spacing.xs))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .child(
                div()
                    .text_color(tokens.colors.text)
                    .child(h_flex().gap_1().child(":").child(echo)),
            )
            .child(
                div()
                    .text_size(u(tokens.font.small))
                    .text_color(color)
                    .child(SharedString::from(problem.message.clone())),
            )
            .into_any_element(),
    )
}

/// The dimmed text at the end of a completion row: what the word stands for.
pub(super) fn detail(text: std::sync::Arc<str>, cx: &App) -> gpui::Div {
    let tokens = cx.tokens();
    div()
        .flex_none()
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .child(SharedString::from(text.to_string()))
}

/// The key hints under the list.
pub(super) fn footer(cx: &App) -> AnyElement {
    let tokens = cx.tokens();
    h_flex()
        .debug_selector(|| "jump-hints".into())
        .flex_none()
        .gap(u(tokens.spacing.lg))
        .px(u(tokens.spacing.lg))
        .py(u(tokens.spacing.sm))
        .border_t_1()
        .border_color(tokens.colors.border_variant)
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .child("Tab complete")
        .child("Enter run")
        .child("- [ ] history")
        .child("Esc close")
        .into_any_element()
}
