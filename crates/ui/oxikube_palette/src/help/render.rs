//! How the overlay draws a group header and a binding.
//!
//! Every row is one picker row high (the list is a `uniform_list`). Colours and sizes come from
//! the tokens; the keys are `oxikube_ui` key caps, drawn with the platform's own symbols.

use gpui::{
    App, Div, ElementId, FontWeight, InteractiveElement as _, ParentElement as _, Role,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _,
};
use oxikube_ui::kbd::keycap;
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, u};

use super::entry::{HelpCategory, HelpEntry, HelpSource, HelpState};
use super::model::HelpScope;
use crate::picker::match_label;

/// A group's heading: the category in capitals, muted, with how many bindings it holds.
pub fn header(category: HelpCategory, count: usize, ix: usize, cx: &App) -> Stateful<Div> {
    let tokens = cx.tokens();
    let label = category.label();
    h_flex()
        .id(ElementId::NamedInteger("help-header".into(), ix as u64))
        .debug_selector(move || format!("help-header-{label}"))
        .role(Role::Heading)
        .aria_label(SharedString::from(format!("{label}, {count} keys")))
        .w_full()
        .gap(u(tokens.spacing.md))
        .items_center()
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(label.to_uppercase()),
        )
        .child(div().child(count.to_string()))
}

/// A binding: its title (the matched characters highlighted), the action's name as secondary
/// text (the key context too when the list is not limited to the focused view), the source chip
/// and the key caps.
pub fn entry(
    entry: &HelpEntry,
    positions: &[usize],
    scope: &HelpScope,
    selected: bool,
    ix: usize,
    cx: &App,
) -> Stateful<Div> {
    let tokens = cx.tokens();
    let colors = tokens.colors;
    let unbound = matches!(entry.state, HelpState::Unbound(_));
    let secondary = match (&entry.context, scope) {
        (Some(context), HelpScope::Everything) => format!("{}  ({context})", entry.action),
        _ => entry.action.to_owned(),
    };
    let keys = entry.keystroke_text();
    let name = SharedString::from(match entry.state {
        HelpState::Active => format!("{}, {keys}", entry.title),
        HelpState::Unbound(by) => format!("{}, {keys}, unbound by {}", entry.title, who(by)),
    });
    let title = match_label(entry.title.clone(), positions, selected, cx)
        .flex_none()
        .when(unbound, |title| title.opacity(0.5));
    let mut row = h_flex()
        .id(ElementId::NamedInteger("help-entry".into(), ix as u64))
        .debug_selector(move || format!("help-entry-{ix}"))
        .role(Role::ListItem)
        .aria_label(name)
        .w_full()
        .gap(u(tokens.spacing.md))
        .items_center()
        .child(title)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(u(tokens.font.small))
                .text_color(colors.text_disabled)
                .child(secondary),
        );
    if let Some(chip) = chip(entry, cx) {
        row = row.child(chip);
    }
    row.child(
        h_flex()
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .opacity(if unbound { 0.5 } else { 1.0 })
            .children(entry.keystrokes.iter().filter_map(|stroke| keycap(stroke))),
    )
}

/// The small label of a binding the user (or the vim base) set, or unbound.
fn chip(entry: &HelpEntry, cx: &App) -> Option<Div> {
    let tokens = cx.tokens();
    let (text, selector) = match entry.state {
        HelpState::Unbound(by) => (format!("Unbound by {}", who(by)), "help-chip-unbound"),
        HelpState::Active => (
            entry.source.chip()?.to_owned(),
            match entry.source {
                HelpSource::User => "help-chip-user",
                _ => "help-chip-base",
            },
        ),
    };
    Some(
        div()
            .debug_selector(move || selector.to_owned())
            .flex_none()
            .px(u(tokens.spacing.sm))
            .rounded(u(tokens.radius.sm))
            .bg(tokens.colors.element)
            .text_size(u(tokens.font.small))
            .text_color(match entry.state {
                HelpState::Unbound(_) => tokens.colors.warning,
                HelpState::Active => tokens.colors.accent,
            })
            .child(text),
    )
}

/// "you" for the user's file, "base" for the vim layer.
fn who(source: HelpSource) -> &'static str {
    match source {
        HelpSource::User => "you",
        HelpSource::Base => "base",
        HelpSource::Default => "default",
    }
}
