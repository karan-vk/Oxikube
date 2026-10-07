//! Mouse reporting: clicks, drags, motion and the wheel go to the process while it asked for them.

use gpui::{Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, TestAppContext, point, px};

use super::{FONT_SIZE, Harness, harness};

/// Click reports (1000) in SGR encoding (1006).
const CLICK_SGR: &str = "\x1b[?1000h\x1b[?1006h";

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("ASCII reports")
}

fn selection(h: &mut Harness) -> Option<String> {
    h.terminal
        .read_with(&mut *h.window, |terminal, _| terminal.selection_text())
}

#[gpui::test]
fn click_and_release_are_reported_in_sgr(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output(CLICK_SGR);
    h.window
        .simulate_mouse_down(Harness::at(2, 3), MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_up(Harness::at(2, 3), MouseButton::Left, Modifiers::none());
    assert_eq!(text(h.written()), "\x1b[<0;4;3M\x1b[<0;4;3m");
    assert_eq!(selection(&mut h), None, "the press belonged to the program");
}

#[gpui::test]
fn other_buttons_are_reported_too(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output(CLICK_SGR);
    for (button, code) in [(MouseButton::Middle, 1), (MouseButton::Right, 2)] {
        h.window
            .simulate_mouse_down(Harness::at(0, 0), button, Modifiers::none());
        h.window
            .simulate_mouse_up(Harness::at(0, 0), button, Modifiers::none());
        let written = text(h.written());
        assert!(
            written.ends_with(&format!("\x1b[<{code};1;1M\x1b[<{code};1;1m")),
            "{written:?}"
        );
    }
}

#[gpui::test]
fn drags_are_reported_once_per_cell_only_in_drag_mode(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    // Click mode only: moving with the button down reports nothing.
    h.output(CLICK_SGR);
    h.window
        .simulate_mouse_down(Harness::at(1, 1), MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_move(Harness::at(1, 5), MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_up(Harness::at(1, 5), MouseButton::Left, Modifiers::none());
    assert_eq!(text(h.written()), "\x1b[<0;2;2M\x1b[<0;6;2m");

    h.output("\x1b[?1002h");
    h.window
        .simulate_mouse_down(Harness::at(1, 1), MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_move(Harness::at(1, 2), MouseButton::Left, Modifiers::none());
    // Inside the same cell: nothing new.
    h.window.simulate_mouse_move(
        Harness::at(1, 2) + point(px(1.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    h.window
        .simulate_mouse_move(Harness::at(1, 3), MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_up(Harness::at(1, 3), MouseButton::Left, Modifiers::none());
    let written = text(h.written());
    assert!(
        written.ends_with("\x1b[<0;2;2M\x1b[<32;3;2M\x1b[<32;4;2M\x1b[<0;4;2m"),
        "{written:?}"
    );
}

#[gpui::test]
fn bare_motion_only_in_all_motion_mode(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("\x1b[?1002h\x1b[?1006h");
    h.window
        .simulate_mouse_move(Harness::at(0, 4), None, Modifiers::none());
    assert_eq!(text(h.written()), "", "1002 reports drags, not bare motion");
    h.output("\x1b[?1003h");
    h.window
        .simulate_mouse_move(Harness::at(0, 5), None, Modifiers::none());
    h.window
        .simulate_mouse_move(Harness::at(0, 6), None, Modifiers::none());
    assert_eq!(text(h.written()), "\x1b[<35;6;1M\x1b[<35;7;1M");
}

#[gpui::test]
fn the_wheel_is_reported(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output(CLICK_SGR);
    h.window.simulate_event(ScrollWheelEvent {
        position: Harness::at(3, 4),
        delta: ScrollDelta::Lines(point(0., 2.)),
        ..Default::default()
    });
    h.window.simulate_event(ScrollWheelEvent {
        position: Harness::at(3, 4),
        delta: ScrollDelta::Lines(point(0., -1.)),
        ..Default::default()
    });
    assert_eq!(text(h.written()), "\x1b[<64;5;4M\x1b[<64;5;4M\x1b[<65;5;4M");
}

#[gpui::test]
fn the_legacy_encoding_when_sgr_is_off(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("\x1b[?1000h");
    h.window
        .simulate_mouse_down(Harness::at(0, 0), MouseButton::Left, Modifiers::none());
    assert_eq!(h.written(), [0x1b, b'[', b'M', 32, 33, 33]);
}

#[gpui::test]
fn shift_bypasses_reporting_for_selection(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output(&format!("{CLICK_SGR}hello world"));
    let shift = Modifiers {
        shift: true,
        ..Modifiers::none()
    };
    let start = Harness::at(0, 0) - point(px(2.), px(0.));
    h.window
        .simulate_mouse_down(start, MouseButton::Left, shift);
    h.window
        .simulate_mouse_move(Harness::at(0, 4), MouseButton::Left, shift);
    h.window
        .simulate_mouse_up(Harness::at(0, 4), MouseButton::Left, shift);
    assert_eq!(selection(&mut h).as_deref(), Some("hello"));
    assert_eq!(text(h.written()), "", "nothing was reported");
}

#[gpui::test]
fn without_a_mouse_mode_the_mouse_stays_local(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("hello world");
    let start = Harness::at(0, 0) - point(px(2.), px(0.));
    h.window
        .simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_move(Harness::at(0, 4), MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_up(Harness::at(0, 4), MouseButton::Left, Modifiers::none());
    assert_eq!(selection(&mut h).as_deref(), Some("hello"));
    assert_eq!(h.written(), b"");
}

#[gpui::test]
fn turning_the_mode_off_returns_the_mouse_to_the_terminal(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output(CLICK_SGR);
    h.output("\x1b[?1000l");
    h.output("hello");
    let start = Harness::at(0, 0) - point(px(2.), px(0.));
    h.window
        .simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_move(Harness::at(0, 2), MouseButton::Left, Modifiers::none());
    assert_eq!(selection(&mut h).as_deref(), Some("hel"));
    assert_eq!(h.written(), b"");
}

fn wheel(h: &mut Harness, lines: f32, modifiers: Modifiers) {
    h.window.simulate_event(ScrollWheelEvent {
        position: Harness::at(3, 4),
        delta: ScrollDelta::Lines(point(0., lines)),
        modifiers,
        ..Default::default()
    });
    h.frame();
}

#[gpui::test]
fn on_the_alternate_screen_the_wheel_sends_cursor_keys(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("\x1b[?1049h");
    wheel(&mut h, 2., Modifiers::none());
    wheel(&mut h, -1., Modifiers::none());
    assert_eq!(text(h.written()), "\x1b[A\x1b[A\x1b[B");
    h.output("\x1b[?1h");
    wheel(&mut h, 1., Modifiers::none());
    assert!(text(h.written()).ends_with("\x1bOA"));
    // Alternate scroll switched off: the wheel is silent.
    h.output("\x1b[?1007l");
    let before = h.written().len();
    wheel(&mut h, 3., Modifiers::none());
    assert_eq!(h.written().len(), before);
}

#[gpui::test]
fn mouse_reporting_wins_over_alternate_scroll(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output(&format!("\x1b[?1049h{CLICK_SGR}"));
    wheel(&mut h, 1., Modifiers::none());
    assert_eq!(text(h.written()), "\x1b[<64;5;4M");
}

#[gpui::test]
fn shift_wheel_scrolls_the_history_even_while_reporting(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    let lines: String = (0..30).map(|i| format!("line {i}\r\n")).collect();
    h.output(&format!("{lines}{CLICK_SGR}"));
    let shift = Modifiers {
        shift: true,
        ..Modifiers::none()
    };
    wheel(&mut h, 2., shift);
    let offset = h
        .terminal
        .read_with(&mut *h.window, |t, _| t.snapshot().display_offset);
    assert_eq!(offset, 2);
    assert_eq!(h.written(), b"");
}
