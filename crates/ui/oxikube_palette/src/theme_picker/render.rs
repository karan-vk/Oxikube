//! How the picker draws a group header, a theme and its caption.
//!
//! Every row is one picker row high (the list is a `uniform_list`).

use gpui::{
    App, Div, ElementId, FontWeight, InteractiveElement as _, ParentElement as _, Role,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div,
};
use oxikube_theme::{Appearance, ThemeChoice, ThemeMode, ThemeSlot};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, u};

use crate::picker::match_label;

/// "Dark themes" or "Light themes".
pub fn group_label(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::Dark => "Dark themes",
        Appearance::Light => "Light themes",
    }
}

/// A group's heading: its label in capitals, muted, with how many themes it holds.
pub fn header(appearance: Appearance, count: usize, ix: usize, cx: &App) -> Stateful<Div> {
    let tokens = cx.tokens();
    let label = group_label(appearance);
    h_flex()
        .id(ElementId::NamedInteger("theme-header".into(), ix as u64))
        .debug_selector(move || format!("theme-header-{}", label.to_lowercase().replace(' ', "-")))
        .role(Role::Heading)
        .aria_label(SharedString::from(format!("{label}, {count}")))
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

/// A theme: its name with the matched characters highlighted, and a "Current" chip on the theme
/// the window shows when the picker opened.
pub fn entry(
    name: &SharedString,
    positions: &[usize],
    current: bool,
    selected: bool,
    ix: usize,
    cx: &App,
) -> Stateful<Div> {
    let tokens = cx.tokens();
    let label = SharedString::from(if current {
        format!("{name}, current theme")
    } else {
        name.to_string()
    });
    let mut row = h_flex()
        .id(ElementId::NamedInteger("theme-entry".into(), ix as u64))
        .debug_selector(move || format!("theme-entry-{ix}"))
        .role(Role::ListItem)
        .aria_label(label)
        .w_full()
        .gap(u(tokens.spacing.md))
        .items_center()
        .child(match_label(name.clone(), positions, selected, cx).flex_1());
    if current {
        row = row.child(
            div()
                .debug_selector(|| "theme-current".to_owned())
                .flex_none()
                .px(u(tokens.spacing.sm))
                .rounded(u(tokens.radius.sm))
                .bg(tokens.colors.element)
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.accent)
                .child("Current"),
        );
    }
    row
}

/// The caption above the list: which part of the `theme` setting a pick edits.
pub fn slot_caption(choice: ThemeChoice) -> String {
    let what = match choice.slot {
        ThemeSlot::Any => "your theme".to_owned(),
        slot => format!("your {}", slot.label()),
    };
    match choice.mode {
        Some(ThemeMode::System) => format!("Editing {what}; mode: system"),
        Some(ThemeMode::Light) => format!("Editing {what}; mode: light"),
        Some(ThemeMode::Dark) => format!("Editing {what}; mode: dark"),
        None => format!("Editing {what}"),
    }
}
