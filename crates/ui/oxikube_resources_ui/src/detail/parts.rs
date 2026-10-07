//! The Overview's rows that need no view state: headings, notes, the conditions table and the
//! `status` lines, as free functions over the model's data.

use gpui::{
    AnyElement, Div, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, div, px,
};
use jiff::Timestamp;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{Tokens, u};

use super::model::{ConditionRow, Section, StatusLine};
use crate::table::ToneColors;

pub(super) fn empty() -> AnyElement {
    div().into_any_element()
}

/// Gives a virtualised list row the list's width, so long values wrap instead of widening it.
pub(super) fn full_width(row: AnyElement) -> AnyElement {
    div().w_full().child(row).into_any_element()
}

pub(super) fn muted(tokens: &Tokens, text: &str) -> AnyElement {
    div()
        .text_color(tokens.colors.text_muted)
        .text_size(u(tokens.font.small))
        .child(text.to_owned())
        .into_any_element()
}

pub(super) fn section_heading(tokens: &Tokens, section: Section, count: usize) -> AnyElement {
    let title = match section {
        Section::Owners | Section::Conditions => section.title().to_owned(),
        _ => format!("{} ({count})", section.title()),
    };
    let id = section.title();
    div()
        .debug_selector(move || format!("detail-section-{id}"))
        .px(u(tokens.spacing.lg))
        .pt(u(tokens.spacing.lg))
        .pb(u(tokens.spacing.sm))
        .text_size(u(tokens.font.small))
        .font_semibold()
        .text_color(tokens.colors.text_muted)
        .child(title.to_uppercase())
        .into_any_element()
}

/// One condition as two lines: the type, its status and how long ago it changed; then, muted and
/// wrapped at word boundaries, the reason and the message. The type never wraps (it is cut with
/// an ellipsis and a tooltip when the drawer is too narrow); the second line is left out when the
/// condition has neither.
pub(super) fn condition_row(
    tokens: &Tokens,
    tones: &ToneColors,
    row: &ConditionRow,
    index: usize,
    now: Timestamp,
) -> AnyElement {
    let changed = row
        .transition
        .map(|at| oxikube_domain::Age::between(at, now).to_kubectl_string())
        .unwrap_or_default();
    let detail = match (row.reason.is_empty(), row.message.is_empty()) {
        (true, true) => None,
        (false, true) => Some(row.reason.clone()),
        (true, false) => Some(row.message.clone()),
        (false, false) => Some(format!("{}: {}", row.reason, row.message)),
    };
    let first = h_flex()
        .items_center()
        .gap(u(tokens.spacing.md))
        .child(
            truncated(
                ("detail-condition-type", index),
                move || format!("detail-condition-type-{index}"),
                row.kind.clone(),
            )
            .flex_1(),
        )
        .child(
            div()
                .debug_selector(move || format!("detail-condition-status-{index}"))
                .flex_none()
                .w(u(px(64.)))
                .text_color(tones.of(row.tone))
                .child(row.status.clone()),
        )
        .child(
            div()
                .debug_selector(move || format!("detail-condition-age-{index}"))
                .flex_none()
                .w(u(px(48.)))
                .text_right()
                .text_color(tokens.colors.text_muted)
                .text_size(u(tokens.font.small))
                .child(changed),
        );
    v_flex()
        .debug_selector(move || format!("detail-condition-{index}"))
        .px(u(tokens.spacing.lg))
        .py(u(px(3.)))
        .child(first)
        .children(detail.map(|text| {
            div()
                .debug_selector(move || format!("detail-condition-detail-{index}"))
                .w_full()
                .whitespace_normal()
                .text_color(tokens.colors.text_muted)
                .text_size(u(tokens.font.small))
                .child(text)
        }))
        .into_any_element()
}

/// One line of text cut with an ellipsis where it does not fit, with the whole text as a tooltip.
/// The caller gives it its width (`flex_1`, or a fixed one).
pub(super) fn truncated(
    id: (&'static str, usize),
    selector: impl Fn() -> String + 'static,
    text: impl Into<SharedString>,
) -> Stateful<Div> {
    let text: SharedString = text.into();
    let tip = text.clone();
    div()
        .id(id)
        .debug_selector(selector)
        .min_w_0()
        .truncate()
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .child(text)
}

/// The width the keys of the Overview's metadata and status rows share, unscaled pixels: the
/// values then start in one column.
pub(super) const KEY_WIDTH: f32 = 150.;

pub(super) fn status_line(tokens: &Tokens, line: &StatusLine, index: usize) -> AnyElement {
    let indent = f32::from(line.depth) * 12.;
    h_flex()
        .debug_selector(move || format!("detail-status-{index}"))
        .items_start()
        .gap(u(tokens.spacing.md))
        .pl(u(tokens.spacing.lg) + u(px(indent)))
        .pr(u(tokens.spacing.lg))
        .py(u(px(1.)))
        .child(
            truncated(
                ("detail-status-key", index),
                move || format!("detail-status-key-{index}"),
                line.key.clone(),
            )
            .flex_none()
            .w(u(px((KEY_WIDTH - indent).max(48.))))
            .text_color(tokens.colors.text_muted),
        )
        .children(line.value.clone().map(|value| {
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_normal()
                .child(value)
        }))
        .into_any_element()
}

/// Grey bars where content is loading.
pub(super) fn skeleton(tokens: &Tokens, selector: &'static str, widths: &[f32]) -> AnyElement {
    v_flex()
        .debug_selector(move || selector.to_owned())
        .gap(u(tokens.spacing.md))
        .p(u(tokens.spacing.xl))
        .children(widths.iter().map(|w| {
            div()
                .h(u(px(10.)))
                .w(u(px(*w)))
                .max_w_full()
                .rounded(u(tokens.radius.sm))
                .bg(tokens.colors.element)
        }))
        .into_any_element()
}
