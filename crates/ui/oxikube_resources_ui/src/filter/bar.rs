//! [`FilterBar`]: the text field of the `/` filter, its parse error and its count.
//!
//! The bar owns the typing; the table owns the rows. Every edit is parsed at once (a pure,
//! once-per-edit compile in `oxikube_app::store::filter`) and the bar shows the first problem
//! next to the field. A good filter reaches the table as [`FilterBarEvent::Changed`], debounced
//! (see `apply`); a bad one is only shown, so the table keeps the rows of the last good filter.

use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Task, Window, div, px,
};
use oxikube_app::store::filter::{FilterError, FilterParts};
use oxikube_ui::input::{Input, InputEvent, InputState};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

/// How long a client-side edit waits for more typing: about one frame at 60 Hz. The first
/// keystroke after a pause is applied at once, so typing feels instant.
pub const DEBOUNCE: Duration = Duration::from_millis(16);

/// How long a label-selector edit waits. A selector is applied by the server: every change opens
/// a new watch, so partial selectors typed on the way are not worth a round trip.
pub const SELECTOR_DEBOUNCE: Duration = Duration::from_millis(250);

/// What a [`FilterBar`] tells its table.
#[derive(Clone, Debug, PartialEq)]
pub enum FilterBarEvent {
    /// The filter to show changed. `text` is what the user typed (for saving).
    Changed {
        /// The parsed filter: the store's part and the server's selector.
        parts: FilterParts,
        /// The text in the bar.
        text: String,
    },
    /// Enter or Escape: give the focus back to the table.
    Returned,
    /// The field gained or lost the focus (the table's key context says `Editing` meanwhile, so
    /// bare keys such as `j` and `/` are text).
    Editing(bool),
}

/// An edit waiting for its debounce.
pub(super) struct Pending {
    pub parts: FilterParts,
    pub text: String,
}

/// The filter bar of one table. See the [module docs](self).
pub struct FilterBar {
    pub(super) input: Entity<InputState>,
    /// The text last parsed: the input's own change event after `set_text` is ignored.
    pub(super) text: String,
    /// The last good filter, as told to the table.
    pub(super) applied: FilterParts,
    pub(super) error: Option<FilterError>,
    pub(super) pending: Option<Pending>,
    /// Whether an edit was applied less than a debounce ago (later ones wait for the timer).
    pub(super) throttled: bool,
    /// The debounce timer; replaced, never cleared from inside itself.
    pub(super) timer: Option<Task<()>>,
    shown: usize,
    total: usize,
    editing: bool,
    _subscription: Subscription,
}

impl EventEmitter<FilterBarEvent> for FilterBar {}

impl FilterBar {
    /// An empty bar.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Filter: name, !not, -l labels, -f fuzzy")
        });
        let subscription = cx.subscribe_in(&input, window, Self::on_input_event);
        Self {
            input,
            text: String::new(),
            applied: FilterParts::default(),
            error: None,
            pending: None,
            throttled: false,
            timer: None,
            shown: 0,
            total: 0,
            editing: false,
            _subscription: subscription,
        }
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => self.on_change(cx),
            InputEvent::PressEnter { .. } => self.commit(window, cx),
            InputEvent::Focus => self.set_editing(true, cx),
            InputEvent::Blur => self.set_editing(false, cx),
        }
    }

    fn set_editing(&mut self, editing: bool, cx: &mut Context<Self>) {
        if self.editing != editing {
            self.editing = editing;
            cx.emit(FilterBarEvent::Editing(editing));
        }
    }

    /// Moves the keyboard focus into the field.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// The text in the bar.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The parse error of the text in the bar, if it has one.
    pub fn error(&self) -> Option<&FilterError> {
        self.error.as_ref()
    }

    /// Whether the field has the keyboard focus.
    pub fn is_editing(&self) -> bool {
        self.editing
    }

    /// Sets how many rows pass the filter of how many there are (`123 of 4,812`). Redraws only
    /// when a number changed and the count is on screen (a filter is on): a feed adding and
    /// deleting objects does not redraw the bar of an unfiltered table.
    pub fn set_counts(&mut self, shown: usize, total: usize, cx: &mut Context<Self>) {
        if (self.shown, self.total) != (shown, total) {
            (self.shown, self.total) = (shown, total);
            if !self.applied.is_empty() {
                cx.notify();
            }
        }
    }

    /// The count as shown: `123 of 4,812` while a filter is on, nothing otherwise.
    pub fn count_label(&self) -> Option<String> {
        (!self.applied.is_empty())
            .then(|| format!("{} of {}", thousands(self.shown), thousands(self.total)))
    }
}

impl Focusable for FilterBar {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Render for FilterBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let count = self.count_label();
        let error: Option<SharedString> = self.error.as_ref().map(|e| e.to_string().into());
        h_flex()
            .id("resource-filter")
            .gap(u(px(6.)))
            .items_center()
            .child(
                div()
                    .text_color(colors.text_muted)
                    .text_size(u(px(12.)))
                    .child("/"),
            )
            .child(
                div()
                    .debug_selector(|| "resource-filter-input".into())
                    .w(u(px(240.)))
                    .child(Input::new(&self.input).xsmall()),
            )
            .children(count.map(|count| {
                div()
                    .debug_selector(|| "resource-filter-count".into())
                    .text_color(colors.text_muted)
                    .text_size(u(px(12.)))
                    .child(SharedString::from(count))
            }))
            .children(error.map(|message| {
                div()
                    .debug_selector(|| "resource-filter-error".into())
                    .max_w(u(px(360.)))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_color(colors.error)
                    .text_size(u(px(12.)))
                    .child(message)
            }))
    }
}

/// `4812` as `4,812`.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::thousands;

    #[test]
    fn counts_get_thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(4_812), "4,812");
        assert_eq!(thousands(10_000), "10,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
