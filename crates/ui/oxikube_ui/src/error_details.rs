//! The "Details" half of an error notice: the toggle and the box with the raw text.
//!
//! The connect view, the log viewer and the terminal show an error as one plain sentence and keep
//! the adapter's own text behind a "Details" toggle. The sentence and the buttons are each view's
//! own; the toggle and the box are shared here so the three look and behave alike. Both are
//! stateless: the view keeps the open flag and flips it in the toggle's handler.

use gpui::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::button::{Button, ButtonVariants as _};
use crate::markdown::MarkdownView;
use crate::{ActiveTokens as _, Sizable as _, u};

/// The label of the toggle: "Details" while collapsed, "Hide details" while open.
pub const fn toggle_label(open: bool) -> &'static str {
    if open { "Hide details" } else { "Details" }
}

/// The Details toggle: a small ghost button that tests find by `selector`.
pub fn details_toggle(
    selector: &'static str,
    open: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let button = Button::new(selector)
        .label(toggle_label(open))
        .small()
        .ghost()
        .on_click(move |_, window, cx| on_click(window, cx));
    div()
        .debug_selector(move || selector.to_owned())
        .child(button)
}

/// The raw text of an error in a box that scrolls past `max_height` (unscaled), selectable with
/// the mouse and copyable with the platform's copy. Drawn as a code block, so error text that
/// looks like Markdown stays text.
pub fn details_box(
    id: impl Into<ElementId>,
    selector: &'static str,
    text: &str,
    max_height: Pixels,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let tokens = cx.tokens();
    let id = id.into();
    div()
        .id(id.clone())
        .debug_selector(move || selector.to_owned())
        .w_full()
        .max_h(u(max_height))
        .overflow_y_scroll()
        .text_size(u(tokens.font.mono))
        .child(
            MarkdownView::markdown(SharedString::from(format!("{id:?}-text")), fenced(text))
                .selectable(true),
        )
}

/// A comfortable height for a details box under a heading: tall enough for a stack of lines.
pub const TALL: Pixels = px(220.);

/// A height for a details box inside a strip that must leave room for what it sits on.
pub const SHORT: Pixels = px(120.);

/// `text` as a Markdown fenced block whose fence no line of the text can close.
fn fenced(text: &str) -> String {
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}text\n{text}\n{fence}")
}

#[cfg(test)]
mod tests {
    use super::{fenced, toggle_label};

    #[test]
    fn the_fence_is_longer_than_any_backtick_run_in_the_text() {
        assert_eq!(fenced("plain"), "```text\nplain\n```");
        assert!(fenced("a ``` b").starts_with("````text"));
        assert!(fenced("`````").starts_with("``````text"));
    }

    #[test]
    fn the_toggle_says_what_it_will_do() {
        assert_eq!(toggle_label(false), "Details");
        assert_eq!(toggle_label(true), "Hide details");
    }
}
