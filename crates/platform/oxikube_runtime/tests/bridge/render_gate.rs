//! `RenderGate` (E05-P599): a streaming view off screen is notified once, not on every frame its
//! stream changes; on screen it is notified once per frame as before.

use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, IntoElement, Render, TestAppContext, Window,
    div,
};
use oxikube_runtime::{RenderGate, notify_pending};
use std::cell::Cell;
use std::rc::Rc;

/// A streaming view: applies events, redraws through its gate.
struct Stream {
    gate: RenderGate,
    events: u64,
    renders: Rc<Cell<u32>>,
}

impl Stream {
    fn event(&mut self, cx: &mut Context<Self>) {
        self.events += 1;
        self.gate.notify(cx);
    }
}

impl Render for Stream {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.gate.rendered();
        self.renders.set(self.renders.get() + 1);
        div()
    }
}

fn stream(_: &mut Context<Stream>) -> Stream {
    Stream {
        gate: RenderGate::default(),
        events: 0,
        renders: Rc::default(),
    }
}

/// Counts the notifies `entity` delivers.
fn observed<T: 'static>(entity: &Entity<T>, cx: &mut TestAppContext) -> Rc<Cell<u32>> {
    let count = Rc::new(Cell::new(0));
    let counter = count.clone();
    cx.update(|cx| {
        cx.observe(entity, move |_, _| counter.set(counter.get() + 1))
            .detach();
    });
    count
}

/// One frame of `window` (its next-frame callbacks, then the draw when dirty).
fn frame(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn an_off_screen_view_is_notified_once_until_it_renders(cx: &mut TestAppContext) {
    // A window that shows something else: the stream view is in none of its frames.
    let window: AnyWindowHandle = cx
        .update(|cx| cx.open_window(Default::default(), |_, cx| cx.new(stream)))
        .unwrap()
        .into();
    cx.run_until_parked();
    let hidden = cx.new(stream);
    let notifies = observed(&hidden, cx);

    for _ in 0..10 {
        hidden.update(cx, |view, cx| {
            view.event(cx);
            view.event(cx);
        });
        frame(window, cx);
    }
    assert_eq!(notifies.get(), 1, "ten frames of changes, one notify");
    assert!(hidden.read_with(cx, |view, _| view.gate.awaiting_render()));
    assert!(!cx.update(|cx| notify_pending(cx, hidden.entity_id())));
    assert_eq!(hidden.read_with(cx, |view, _| view.events), 20);

    // Shown (rendered) again: the next change notifies.
    hidden.update(cx, |view, _| view.gate.rendered());
    hidden.update(cx, |view, cx| view.event(cx));
    frame(window, cx);
    assert_eq!(notifies.get(), 2);
}

#[gpui::test]
fn an_on_screen_view_is_notified_once_per_frame(cx: &mut TestAppContext) {
    let window = cx
        .update(|cx| cx.open_window(Default::default(), |_, cx| cx.new(stream)))
        .unwrap();
    cx.run_until_parked();
    let view = window.update(cx, |_, _, cx| cx.entity()).unwrap();
    let renders = view.read_with(cx, |view, _| view.renders.clone());
    let notifies = observed(&view, cx);
    let window: AnyWindowHandle = window.into();
    let renders_before = renders.get();

    for frame_index in 1..=5 {
        for _ in 0..100 {
            view.update(cx, |view, cx| view.event(cx));
        }
        frame(window, cx);
        assert_eq!(notifies.get(), frame_index, "one notify per frame");
        assert_eq!(
            renders.get() - renders_before,
            frame_index,
            "drawn in that frame"
        );
        assert!(!view.read_with(cx, |view, _| view.gate.awaiting_render()));
    }
}
