//! The small stateless pieces the Network panels are made of: headings, fields, chips, table
//! rows and condition lines.

use gpui::prelude::*;
use gpui::{AnyElement, Div, ElementId, SharedString, Stateful, div, px, relative};
use oxikube_app::columns::Tone;
use oxikube_domain::view::Condition;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, Tokens, u};

use crate::table::ToneColors;

/// The most rows of a table a panel draws; the rest is counted ("and 12 more"). The full object
/// stays in the YAML tab.
pub(super) const MAX_ROWS: usize = 100;

/// The theme values one frame of a panel needs.
pub(super) struct Look {
    pub(super) tokens: Tokens,
    pub(super) tones: ToneColors,
}

impl Look {
    pub(super) fn of(cx: &gpui::App) -> Self {
        Self {
            tokens: cx.tokens(),
            tones: ToneColors::current(cx),
        }
    }
}

/// The panel's outer box.
pub(super) fn shell(look: &Look, id: &'static str) -> Div {
    v_flex()
        .debug_selector(move || id.to_owned())
        .w_full()
        .gap(u(look.tokens.spacing.sm))
        .px(u(look.tokens.spacing.lg))
        .py(u(look.tokens.spacing.md))
}

/// A section heading in small capitals: `RULES (3)`.
pub(super) fn heading(look: &Look, id: &'static str, title: &str, count: Option<usize>) -> Div {
    let text = match count {
        Some(n) => format!("{title} ({n})"),
        None => title.to_owned(),
    };
    div()
        .debug_selector(move || id.to_owned())
        .pt(u(look.tokens.spacing.md))
        .text_size(u(look.tokens.font.small))
        .font_semibold()
        .text_color(look.tokens.colors.text_muted)
        .child(text.to_uppercase())
}

/// Muted explanatory text.
pub(super) fn note(look: &Look, id: &'static str, text: impl Into<SharedString>) -> Div {
    div()
        .debug_selector(move || id.to_owned())
        .w_full()
        .whitespace_normal()
        .text_size(u(look.tokens.font.small))
        .text_color(look.tokens.colors.text_muted)
        .child(text.into())
}

/// `key  value`: the key muted in a fixed column, the value wrapping beside it.
pub(super) fn field(
    look: &Look,
    id: &'static str,
    key: &str,
    value: impl IntoElement,
) -> AnyElement {
    h_flex()
        .debug_selector(move || id.to_owned())
        .items_start()
        .gap(u(look.tokens.spacing.md))
        .child(
            div()
                .flex_none()
                .w(u(px(110.)))
                .text_color(look.tokens.colors.text_muted)
                .child(key.to_owned()),
        )
        .child(div().flex_1().min_w_0().whitespace_normal().child(value))
        .into_any_element()
}

/// A small pill in the colour of `tone`, as its own row so that it is as wide as its text, not
/// as the box it is in.
pub(super) fn chip(
    look: &Look,
    id: &'static str,
    text: impl Into<SharedString>,
    tone: Tone,
) -> Div {
    let color = look.tones.of(tone);
    h_flex().child(
        div()
            .debug_selector(move || id.to_owned())
            .flex_none()
            .px(u(look.tokens.spacing.md))
            .rounded(u(look.tokens.radius.sm))
            .bg(color.opacity(0.14))
            .text_color(color)
            .text_size(u(look.tokens.font.small))
            .child(text.into()),
    )
}

/// A link to another object: accent-coloured and underlined on hover when it `enabled`
/// (the caller adds `on_click`), plain muted text when it cannot be followed yet.
pub(super) fn link(
    look: &Look,
    id: impl Into<ElementId>,
    selector: String,
    text: impl Into<SharedString>,
    enabled: bool,
) -> Stateful<Div> {
    let colors = look.tokens.colors;
    let text: SharedString = text.into();
    let tip = text.clone();
    div()
        .id(id)
        .debug_selector(move || selector)
        .min_w_0()
        .truncate()
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .when(enabled, |el| {
            el.cursor_pointer()
                .text_color(colors.accent)
                .hover(|style| style.underline())
        })
        .when(!enabled, |el| el.text_color(colors.text_muted))
        .child(text)
}

/// A cell of a table row: `fraction` of the row's width, text cut with an ellipsis.
pub(super) fn cell(fraction: f32, content: impl IntoElement) -> Div {
    div()
        .flex_none()
        .w(relative(fraction))
        .min_w_0()
        .pr(u(px(6.)))
        .truncate()
        .child(content)
}

/// A body row of a table: its [`cell`]s side by side.
pub(super) fn table_row(id: String) -> Div {
    h_flex()
        .debug_selector(move || id)
        .w_full()
        .items_center()
        .py(u(px(2.)))
}

/// The header row of a table: `(title, fraction)` per column.
pub(super) fn table_header(look: &Look, id: &'static str, columns: &[(&str, f32)]) -> Div {
    h_flex()
        .debug_selector(move || id.to_owned())
        .w_full()
        .text_size(u(look.tokens.font.small))
        .text_color(look.tokens.colors.text_muted)
        .border_b_1()
        .border_color(look.tokens.colors.border_variant)
        .children(
            columns
                .iter()
                .map(|(title, fraction)| cell(*fraction, title.to_string())),
        )
}

/// The line after a table that was cut at [`MAX_ROWS`].
pub(super) fn more_rows(look: &Look, hidden: usize) -> Option<Div> {
    (hidden > 0).then(|| {
        note(
            look,
            "net-more-rows",
            format!("{hidden} more rows are not shown; the YAML tab has them all."),
        )
    })
}

/// One condition: type, status and the reason, then the message wrapped below.
pub(super) fn condition_line(look: &Look, id: String, condition: &Condition) -> AnyElement {
    let color = look.tones.of(Tone::of_condition(condition.status));
    let reason = condition.reason.as_deref().unwrap_or_default().to_owned();
    v_flex()
        .debug_selector(move || id)
        .child(
            h_flex()
                .gap(u(look.tokens.spacing.md))
                .child(div().flex_none().child(condition.kind.to_string()))
                .child(
                    div()
                        .flex_none()
                        .text_color(color)
                        .child(condition.status.as_str()),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(look.tokens.colors.text_muted)
                        .child(reason),
                ),
        )
        .children(condition.message.as_ref().map(|message| {
            div()
                .w_full()
                .whitespace_normal()
                .pl(u(look.tokens.spacing.lg))
                .text_size(u(look.tokens.font.small))
                .text_color(look.tokens.colors.text_muted)
                .child(message.to_string())
        }))
        .into_any_element()
}

/// The conditions as lines, or a note when the controller has reported none.
pub(super) fn conditions(
    look: &Look,
    prefix: &'static str,
    conditions: &[Condition],
) -> Vec<AnyElement> {
    if conditions.is_empty() {
        return vec![
            note(
                look,
                prefix,
                "No status reported yet: no controller has handled this object.",
            )
            .into_any_element(),
        ];
    }
    conditions
        .iter()
        .enumerate()
        .map(|(i, c)| condition_line(look, format!("{prefix}-{i}"), c))
        .collect()
}
