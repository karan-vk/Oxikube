//! Drawing the states that have no rows, and the stale badge above rows that may be old.
//!
//! Everything here is a pure function of the state, the labels and one flag: no data work, one
//! small element tree. The only motion is the spinner, which stands still under reduce-motion and
//! is only built while the table is on screen (a hidden tab does not render).

use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::copy::{StateCopy, StateLabels, copy, stale_label, stale_tip};
use super::state::{Stale, TableState};
use crate::table::view::ResourceTable;

/// Rows of the loading skeleton and the width of each of its three bars, in percent of a row.
const SKELETON: [[f32; 3]; 7] = [
    [22.0, 14.0, 9.0],
    [30.0, 10.0, 12.0],
    [18.0, 16.0, 8.0],
    [26.0, 12.0, 11.0],
    [20.0, 15.0, 7.0],
    [28.0, 9.0, 13.0],
    [16.0, 13.0, 10.0],
];

/// The view of a table with no rows in `state`. `details_open` says whether the failure detail
/// is expanded; `view` receives the buttons' clicks.
pub fn state_view(
    state: &TableState,
    labels: &StateLabels,
    details_open: bool,
    view: &WeakEntity<ResourceTable>,
    cx: &App,
) -> AnyElement {
    let colors = cx.colors();
    let words = copy(state, labels);
    let accent = icon_colour(state, cx);
    let skeleton = matches!(state, TableState::Loading);

    let mut column = v_flex()
        .id("resource-table-state")
        .debug_selector(|| "resource-table-state".into())
        .size_full()
        .items_center()
        .justify_center()
        .gap(u(px(10.)))
        .p(u(px(24.)))
        .text_color(colors.text);
    if skeleton {
        column = column.child(skeleton_rows(cx));
    }
    column = column
        .child(icon_or_spinner(state, &words, accent, cx))
        .child(
            div()
                .debug_selector(|| "resource-table-state-title".into())
                .text_size(u(px(14.)))
                .text_color(colors.text)
                .child(SharedString::from(words.title.clone())),
        );
    if let Some(hint) = &words.hint {
        column = column.child(muted_line("resource-table-state-hint", hint, cx));
    }
    if let Some(summary) = &words.summary {
        column = column.child(muted_line("resource-table-state-summary", summary, cx));
    }
    column = column.child(buttons(state, words.detail.is_some(), details_open, view));
    if details_open && let Some(detail) = &words.detail {
        column = column.child(
            div()
                .debug_selector(|| "resource-table-state-detail".into())
                .max_w(u(px(620.)))
                .max_h(u(px(160.)))
                .overflow_hidden()
                .p(u(px(8.)))
                .rounded(u(px(4.)))
                .bg(colors.surface)
                .border_1()
                .border_color(colors.border_variant)
                .text_size(u(px(11.)))
                .text_color(colors.text_muted)
                .child(SharedString::from(detail.clone())),
        );
    }
    column.into_any_element()
}

/// One centred line of muted text (the hint, the failure summary).
fn muted_line(selector: &'static str, text: &str, cx: &App) -> AnyElement {
    div()
        .debug_selector(move || selector.into())
        .max_w(u(px(520.)))
        .text_center()
        .text_size(u(px(12.)))
        .text_color(cx.colors().text_muted)
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

fn icon_colour(state: &TableState, cx: &App) -> gpui::Hsla {
    let colors = cx.colors();
    match state {
        TableState::Reconnecting { .. }
        | TableState::Forbidden { .. }
        | TableState::Unauthorized { .. } => colors.warning,
        TableState::Failed { .. } => colors.error,
        TableState::Loading
        | TableState::Empty
        | TableState::FilteredEmpty { .. }
        | TableState::Rows { .. } => colors.text_muted,
    }
}

fn icon_or_spinner(
    state: &TableState,
    words: &StateCopy,
    colour: gpui::Hsla,
    cx: &App,
) -> AnyElement {
    if state.is_busy() {
        return div()
            .debug_selector(|| "resource-table-spinner".into())
            .child(
                Spinner::new()
                    .icon(Icon::new(IconName::LoaderCircle))
                    .large()
                    .color(cx.colors().accent),
            )
            .into_any_element();
    }
    div()
        .debug_selector(|| "resource-table-state-icon".into())
        .child(Icon::new(words.icon).size(u(px(28.))).color(colour))
        .into_any_element()
}

/// Placeholder rows: three muted bars each, static, so loading costs one small element tree.
fn skeleton_rows(cx: &App) -> AnyElement {
    let bar = cx.colors().border_variant;
    v_flex()
        .debug_selector(|| "resource-table-skeleton".into())
        .w_full()
        .max_w(u(px(720.)))
        .gap(u(px(10.)))
        .pb(u(px(12.)))
        .children(SKELETON.iter().map(|widths| {
            h_flex()
                .w_full()
                .gap(u(px(24.)))
                .children(widths.iter().map(|w| {
                    div()
                        .h(u(px(10.)))
                        .w(gpui::relative(w / 100.0))
                        .rounded(u(px(3.)))
                        .bg(bar)
                }))
        }))
        .into_any_element()
}

/// `button` in a box tests can find (`Button` has no selector of its own).
fn selectable(selector: &'static str, button: Button) -> AnyElement {
    div()
        .debug_selector(move || selector.into())
        .child(button)
        .into_any_element()
}

/// Retry, Clear filter and the details toggle, whichever the state offers.
fn buttons(
    state: &TableState,
    has_detail: bool,
    details_open: bool,
    view: &WeakEntity<ResourceTable>,
) -> AnyElement {
    let mut row = h_flex().gap(u(px(8.))).items_center();
    if state.can_retry() {
        row = row.child(selectable("resource-table-retry", retry_button(view)));
    }
    if matches!(state, TableState::FilteredEmpty { .. }) {
        let view = view.clone();
        row = row.child(selectable(
            "resource-table-clear-filter",
            Button::new("resource-table-clear-filter")
                .label("Clear filter")
                .icon(Icon::new(IconName::X))
                .primary()
                .small()
                .on_click(move |_, window, cx| {
                    view.update(cx, |table, cx| table.clear_filter(window, cx))
                        .ok();
                }),
        ));
    }
    if has_detail {
        let view = view.clone();
        row = row.child(selectable(
            "resource-table-details-toggle",
            Button::new("resource-table-details-toggle")
                .label(if details_open {
                    "Hide details"
                } else {
                    "Details"
                })
                .ghost()
                .small()
                .on_click(move |_, _, cx| {
                    view.update(cx, |table, cx| table.toggle_state_details(cx))
                        .ok();
                }),
        ));
    }
    row.into_any_element()
}

fn retry_button(view: &WeakEntity<ResourceTable>) -> Button {
    let view = view.clone();
    Button::new("resource-table-retry")
        .label("Retry")
        .icon(Icon::new(IconName::RefreshCw))
        .primary()
        .small()
        .on_click(move |_, _, cx| {
            view.update(cx, |table, cx| table.request_retry(cx)).ok();
        })
}

/// The badge above rows that may be old: what is stale, with a "Retry" when that helps.
pub fn stale_badge(
    stale: &Stale,
    retry: bool,
    view: &WeakEntity<ResourceTable>,
    cx: &App,
) -> AnyElement {
    let colors = cx.colors();
    let mut badge = h_flex()
        .id("resource-table-stale")
        .debug_selector(|| "resource-table-stale".into())
        .gap(u(px(6.)))
        .items_center()
        .px(u(px(6.)))
        .h(u(px(20.)))
        .rounded(u(px(10.)))
        .bg(colors.element)
        .border_1()
        .border_color(colors.border_variant)
        .text_size(u(px(11.)))
        .text_color(colors.warning)
        .tooltip({
            let tip = SharedString::from(stale_tip(stale));
            move |window, cx| Tooltip::new(tip.clone()).build(window, cx)
        })
        .child(if stale.is_busy() {
            Spinner::new()
                .icon(Icon::new(IconName::LoaderCircle))
                .xsmall()
                .color(colors.warning)
                .into_any_element()
        } else {
            Icon::new(IconName::TriangleAlert)
                .size(u(px(12.)))
                .color(colors.warning)
                .into_any_element()
        })
        .child(stale_label(stale));
    if retry {
        let view = view.clone();
        badge = badge.child(selectable(
            "resource-table-stale-retry",
            Button::new("resource-table-stale-retry")
                .label("Retry")
                .ghost()
                .xsmall()
                .on_click(move |_, _, cx| {
                    view.update(cx, |table, cx| table.request_retry(cx)).ok();
                }),
        ));
    }
    badge.into_any_element()
}
