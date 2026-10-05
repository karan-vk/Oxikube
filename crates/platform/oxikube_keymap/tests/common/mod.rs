//! Shared fixtures: test actions and a probe view that records the actions it receives.
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    Action, App, Context, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, Window, WindowHandle, actions, div,
};
use schemars::JsonSchema;
use serde::Deserialize;

// Actions the embedded default keymaps name: registering them here makes those bindings live.
actions!(oxikube, [Quit, Hide]);
actions!(palette, [Toggle]);
// Actions of the vim layer.
actions!(table, [SelectNext, SelectPrevious]);
// Plain test actions.
actions!(kmtest, [Alpha, Beta, Gamma]);

/// An action with data.
#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = kmtest)]
pub struct Scale {
    pub replicas: u32,
}

pub type Log = Rc<RefCell<Vec<String>>>;

/// A focusable view with one key context that logs every action it receives.
pub struct Probe {
    focus: FocusHandle,
    context: &'static str,
    log: Log,
}

impl Focusable for Probe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

macro_rules! log_action {
    ($cx:expr, $ty:ty) => {
        $cx.listener(|this: &mut Probe, _: &$ty, _, _| {
            this.log.borrow_mut().push(stringify!($ty).to_owned())
        })
    };
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.focus)
            .key_context(self.context)
            .on_action(log_action!(cx, Quit))
            .on_action(log_action!(cx, Hide))
            .on_action(log_action!(cx, Toggle))
            .on_action(log_action!(cx, SelectNext))
            .on_action(log_action!(cx, SelectPrevious))
            .on_action(log_action!(cx, Alpha))
            .on_action(log_action!(cx, Beta))
            .on_action(log_action!(cx, Gamma))
            .on_action(cx.listener(|this, action: &Scale, _, _| {
                this.log
                    .borrow_mut()
                    .push(format!("Scale({})", action.replicas));
            }))
            .child("probe")
    }
}

/// Open a window whose focused view has the key context `context`.
pub fn probe(cx: &mut TestAppContext, context: &'static str) -> (WindowHandle<Probe>, Log) {
    let log = Log::default();
    let handle = cx.add_window({
        let log = log.clone();
        move |window, cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            Probe {
                focus,
                context,
                log,
            }
        }
    });
    cx.run_until_parked();
    (handle, log)
}

/// Type `keystrokes` into the probe window and return what the probe logged since last call.
pub fn press(
    cx: &mut TestAppContext,
    window: WindowHandle<Probe>,
    log: &Log,
    keystrokes: &str,
) -> Vec<String> {
    log.borrow_mut().clear();
    cx.simulate_keystrokes(window.into(), keystrokes);
    cx.run_until_parked();
    std::mem::take(&mut *log.borrow_mut())
}
