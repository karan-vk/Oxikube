//! Getting an edit to the table: parse, debounce, apply.
//!
//! The first keystroke after a pause is applied at once (the filter shows in the next frame),
//! later ones inside the debounce window collapse into one application when it ends. A label
//! selector waits longer (and only trailing): it is applied by the server, and a partial
//! selector is not worth a watch. Enter and Escape apply at once.

use std::time::Duration;

use gpui::{Context, Window};
use oxikube_app::search::filter::FilterParts;

use super::bar::{DEBOUNCE, FilterBar, FilterBarEvent, Pending, SELECTOR_DEBOUNCE};

impl FilterBar {
    /// The input's text changed: parse it, show the error or queue the filter.
    pub(super) fn on_change(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        if text == self.state.text() {
            return;
        }
        if self.state.edit(&text) {
            self.queue(self.state.parts().clone(), text, cx);
        }
        cx.notify();
    }

    fn queue(&mut self, parts: FilterParts, text: String, cx: &mut Context<Self>) {
        let server_side = parts.selector != self.applied.selector;
        self.pending = Some(Pending { parts, text });
        if server_side {
            self.start_timer(SELECTOR_DEBOUNCE, cx);
        } else if self.throttled {
            // The timer of the last application is running: this edit goes out when it ends.
        } else {
            self.flush(cx);
            self.throttled = true;
            self.start_timer(DEBOUNCE, cx);
        }
    }

    /// Replaces the timer with one that applies the pending edit after `delay`. (Replaces it from
    /// the outside only: the task never clears its own handle.)
    fn start_timer(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let timer = cx.background_executor().timer(delay);
        self.timer = Some(cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |bar, cx| {
                bar.throttled = false;
                bar.flush(cx);
            })
            .ok();
        }));
    }

    /// Tells the table about the pending edit, if there is one.
    pub(super) fn flush(&mut self, cx: &mut Context<Self>) {
        let Some(Pending { parts, text }) = self.pending.take() else {
            return;
        };
        if parts == self.applied {
            return;
        }
        self.applied = parts.clone();
        self.applied_text.clone_from(&text);
        cx.emit(FilterBarEvent::Changed { parts, text });
        cx.notify();
    }

    /// Enter: applies what is typed now and returns the focus to the table (the text stays).
    pub(super) fn commit(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.settle(cx);
        cx.emit(FilterBarEvent::Returned);
    }

    /// Applies the pending edit now, cancelling the debounce.
    fn settle(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        self.throttled = false;
        self.flush(cx);
    }

    /// Escape: clears the text and the filter and returns the focus to the table.
    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_text("", window, cx);
        cx.emit(FilterBarEvent::Returned);
    }

    /// Puts `text` in the bar and applies it at once, as if typed and confirmed (restoring a
    /// saved filter, or a test). Without a filter the table shows every row.
    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        if self.state.edit(text) {
            self.pending = Some(Pending {
                parts: self.state.parts().clone(),
                text: text.to_owned(),
            });
            self.settle(cx);
        }
        cx.notify();
    }
}
