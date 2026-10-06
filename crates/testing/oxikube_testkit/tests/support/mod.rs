//! A small view the harness tests (and the docs) use as the "thing under test".
//!
//! `Counter` has one key context, two actions, a focus handle and a debounced task: every moving
//! part a UI test drives (keymap, action dispatch, focus, the clock). It follows the task rules
//! of `docs/testing-gpui.md`: the debounce `Task` is stored in a field and replaced from the
//! outside, never cleared from inside itself.

#![allow(dead_code)]

use std::time::Duration;

use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Render, Styled as _, Task, Window, actions, div, px, rgb,
};

actions!(
    harness_example,
    [
        /// Adds one to the counter and (re)starts the settle debounce.
        Increment,
        /// Sets the counter back to zero.
        Reset,
    ]
);

/// How long the counter waits after the last [`Increment`] before it reports `settled`.
pub const SETTLE_AFTER: Duration = Duration::from_millis(300);

/// The bindings the example uses: `j` increments, `escape` resets.
pub fn bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("j", Increment, Some("Counter")),
        KeyBinding::new("escape", Reset, Some("Counter")),
    ]
}

/// See the [module docs](self).
pub struct Counter {
    focus: FocusHandle,
    pub count: u32,
    pub settled: bool,
    /// The running debounce. Replacing it cancels the previous one (dropping a `Task` cancels it).
    debounce: Option<Task<()>>,
}

impl Counter {
    /// A focused counter.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            focus,
            count: 0,
            settled: true,
            debounce: None,
        }
    }

    fn increment(&mut self, _: &Increment, _: &mut Window, cx: &mut Context<Self>) {
        self.count += 1;
        self.settled = false;
        // The task is stored from the outside and only ever replaced by the next increment.
        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE_AFTER).await;
            this.update(cx, |this, cx| {
                this.settled = true;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn reset(&mut self, _: &Reset, _: &mut Window, cx: &mut Context<Self>) {
        self.count = 0;
        self.settled = true;
        self.debounce = None;
        cx.notify();
    }
}

impl Focusable for Counter {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Counter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Counter")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::increment))
            .on_action(cx.listener(Self::reset))
            .size_full()
            .bg(rgb(0x1e2430))
            .text_color(rgb(0xe6edf3))
            .p(px(16.))
            .child(
                div()
                    .debug_selector(|| "count".into())
                    .child(format!("count {}", self.count)),
            )
            .child(if self.settled { "settled" } else { "pending" })
    }
}
