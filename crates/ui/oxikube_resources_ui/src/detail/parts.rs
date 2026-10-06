//! The Overview's rows that need no view state: headings, notes, the conditions table and the
//! `status` lines, as free functions over the model's data.

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, div, px,
};
use jiff::Timestamp;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
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

/// The conditions table's column widths, in percent of the row: type, status, reason, message,
/// last transition (the rest is the gaps).
const CONDITION_COLUMNS: [f32; 5] = [19., 9., 21., 32., 12.];

fn cell(percent: f32, child: impl IntoElement) -> impl IntoElement {
    div()
        .flex_none()
        .min_w_0()
        .w(gpui::relative(percent / 100.))
        .child(child)
}

pub(super) fn condition_head(tokens: &Tokens) -> AnyElement {
    let titles = ["Type", "Status", "Reason", "Message", "Changed"];
    h_flex()
        .debug_selector(|| "detail-condition-head".to_owned())
        .gap(u(tokens.spacing.md))
        .px(u(tokens.spacing.lg))
        .py(u(px(2.)))
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .border_b_1()
        .border_color(tokens.colors.border_variant)
        .children(
            titles
                .into_iter()
                .zip(CONDITION_COLUMNS)
                .map(|(title, weight)| cell(weight, title)),
        )
        .into_any_element()
}

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
    h_flex()
        .debug_selector(move || format!("detail-condition-{index}"))
        .items_start()
        .gap(u(tokens.spacing.md))
        .px(u(tokens.spacing.lg))
        .py(u(px(2.)))
        .child(cell(CONDITION_COLUMNS[0], row.kind.clone()))
        .child(cell(
            CONDITION_COLUMNS[1],
            div()
                .text_color(tones.of(row.tone))
                .child(row.status.clone()),
        ))
        .child(cell(CONDITION_COLUMNS[2], row.reason.clone()))
        .child(cell(
            CONDITION_COLUMNS[3],
            div().whitespace_normal().child(row.message.clone()),
        ))
        .child(cell(CONDITION_COLUMNS[4], changed))
        .into_any_element()
}

pub(super) fn status_line(tokens: &Tokens, line: &StatusLine, index: usize) -> AnyElement {
    let indent = f32::from(line.depth) * 12.;
    h_flex()
        .debug_selector(move || format!("detail-status-{index}"))
        .items_start()
        .gap(u(tokens.spacing.md))
        .pl(u(px(12. + indent)))
        .pr(u(tokens.spacing.lg))
        .py(u(px(1.)))
        .child(
            div()
                .flex_none()
                .text_color(tokens.colors.text_muted)
                .child(line.key.clone()),
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
