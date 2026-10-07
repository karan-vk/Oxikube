//! Selection drags and wheel scrolling.

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

    // On the alternate screen the wheel belongs to the application.
    h.output("\x1b[?1049h");
    h.window.simulate_event(ScrollWheelEvent {
        position: Harness::at(3, 3),
        delta: ScrollDelta::Lines(point(0., 2.)),
        ..Default::default()
    });
    h.frame();
    let offset = h.terminal.read_with(&mut *h.window, |terminal, _| {
        terminal.snapshot().display_offset
    });
    assert_eq!(offset, 0);
}
