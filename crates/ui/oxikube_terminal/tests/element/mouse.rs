//! Selection drags and wheel scrolling.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, TestAppContext, point, px};

use super::{FONT_SIZE, Harness, ROW, harness};

#[gpui::test]
fn a_drag_selects_cells(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("hello world");
    let start = Harness::at(0, 0) - point(px(2.), px(0.));
    h.window
        .simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_move(Harness::at(0, 4), MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_up(Harness::at(0, 4), MouseButton::Left, Modifiers::none());
    let text = h
        .terminal
        .read_with(&mut *h.window, |terminal, _| terminal.selection_text());
    assert_eq!(text.as_deref(), Some("hello"));
}

#[gpui::test]
fn the_wheel_scrolls_through_the_history(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    let lines: String = (0..30).map(|i| format!("line {i}\r\n")).collect();
    h.output(&lines);
    h.window.simulate_event(ScrollWheelEvent {
        position: Harness::at(3, 3),
        delta: ScrollDelta::Pixels(point(px(0.), px(ROW * 3. + 4.))),
        ..Default::default()
    });
    h.frame();
    let snapshot = h
        .terminal
        .read_with(&mut *h.window, |terminal, _| terminal.snapshot());
    assert_eq!(snapshot.display_offset, 3);
    assert_eq!(snapshot.row_text(0), "line 18");
}

/// Counts the terminal's notifications: every `TerminalState::scroll` notifies, even one the grid
/// clamps to nothing (the alternate screen has no history), so this sees a scroll the display
/// offset cannot.
fn count_notifies(h: &mut Harness) -> Rc<Cell<u32>> {
    let notifies = Rc::new(Cell::new(0));
    let (terminal, counter) = (h.terminal.clone(), notifies.clone());
    h.window.update(|_, cx| {
        cx.observe(&terminal, move |_, _| counter.set(counter.get() + 1))
            .detach();
    });
    notifies
}

fn wheel_lines(h: &mut Harness, lines: f32) {
    h.window.simulate_event(ScrollWheelEvent {
        position: Harness::at(3, 3),
        delta: ScrollDelta::Lines(point(0., lines)),
        ..Default::default()
    });
    h.frame();
}

#[gpui::test]
fn on_the_alternate_screen_the_wheel_belongs_to_the_application(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    let lines: String = (0..30).map(|i| format!("line {i}\r\n")).collect();
    h.output(&lines);
    let notifies = count_notifies(&mut h);

    // Control: on the primary screen the same wheel event scrolls the terminal.
    wheel_lines(&mut h, 2.);
    let offset = h.terminal.read_with(&mut *h.window, |terminal, _| {
        terminal.snapshot().display_offset
    });
    assert_eq!(offset, 2);
    assert!(notifies.get() > 0, "a primary-screen scroll notifies");

    // On the alternate screen the element leaves the terminal alone: no scroll, no notify.
    h.output("\x1b[?1049h");
    notifies.set(0);
    wheel_lines(&mut h, 2.);
    wheel_lines(&mut h, -2.);
    assert_eq!(notifies.get(), 0, "the element did not scroll the terminal");
}
