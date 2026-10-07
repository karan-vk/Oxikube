//! Typing: keystrokes become the bytes the process expects, through the focused element.

use gpui::{KeyDownEvent, Keystroke, Modifiers, TestAppContext};

use super::{FONT_SIZE, harness};
use crate::configure;

fn keys(cx: &mut TestAppContext, keystrokes: &str) -> Vec<u8> {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.window.simulate_keystrokes(keystrokes);
    h.written()
}

#[gpui::test]
fn typed_text_reaches_the_process_as_utf8(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.window.simulate_input("ls é");
    assert_eq!(h.written(), "ls é".as_bytes());
}

#[gpui::test]
fn arrows_follow_application_cursor_keys(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.window.simulate_keystrokes("up left");
    assert_eq!(h.written(), b"\x1b[A\x1b[D");
    // The process switches DECCKM on (vim, less): the same keys now send SS3.
    h.output("\x1b[?1h");
    h.window.simulate_keystrokes("up left");
    assert_eq!(h.written(), b"\x1b[A\x1b[D\x1bOA\x1bOD");
    // And off again.
    h.output("\x1b[?1l");
    h.window.simulate_keystrokes("down");
    assert!(h.written().ends_with(b"\x1b[B"));
}

#[gpui::test]
fn modified_navigation_keys(cx: &mut TestAppContext) {
    assert_eq!(keys(cx, "ctrl-right"), b"\x1b[1;5C");
    assert_eq!(keys(cx, "shift-up"), b"\x1b[1;2A");
    assert_eq!(keys(cx, "home end"), b"\x1b[H\x1b[F");
    assert_eq!(keys(cx, "delete insert"), b"\x1b[3~\x1b[2~");
    assert_eq!(keys(cx, "f1 f5 f12"), b"\x1bOP\x1b[15~\x1b[24~");
}

#[gpui::test]
fn enter_tab_escape_and_backspace(cx: &mut TestAppContext) {
    assert_eq!(keys(cx, "enter"), b"\r");
    assert_eq!(keys(cx, "tab"), b"\t");
    assert_eq!(keys(cx, "shift-tab"), b"\x1b[Z");
    assert_eq!(keys(cx, "escape"), b"\x1b");
    assert_eq!(keys(cx, "backspace"), b"\x7f");
}

#[gpui::test]
fn ctrl_c_is_an_interrupt_even_with_a_selection(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("hello world");
    h.terminal.update(&mut *h.window, |terminal, cx| {
        use oxikube_terminal::{GridPoint, SelectionKind, SelectionSide};
        terminal.start_selection(
            SelectionKind::Cell,
            GridPoint::new(0, 0),
            SelectionSide::Left,
            cx,
        );
        terminal.update_selection(GridPoint::new(0, 4), SelectionSide::Right, cx);
    });
    h.window.simulate_keystrokes("ctrl-c");
    assert_eq!(h.written(), b"\x03");
    h.window.simulate_keystrokes("ctrl-d ctrl-z ctrl-l");
    assert_eq!(h.written(), b"\x03\x04\x1a\x0c");
}

#[gpui::test]
fn alt_is_meta_only_when_the_setting_says_so(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "option_as_meta": true } }"#);
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.window.simulate_keystrokes("alt-b alt-f");
    assert_eq!(h.written(), b"\x1bb\x1bf");

    // Switching it off applies to the next keystroke: Option is text again (nothing to send
    // here, the platform would deliver the composed character).
    configure(cx, r#"{ "terminal": { "option_as_meta": false } }"#);
    h.window.simulate_keystrokes("alt-b");
    assert_eq!(h.written(), b"\x1bb\x1bf");
    // Control codes with alt always carry the prefix.
    h.window.simulate_keystrokes("ctrl-alt-a");
    assert_eq!(h.written(), b"\x1bb\x1bf\x1b\x01");
}

#[gpui::test]
fn platform_shortcuts_are_not_sent(cx: &mut TestAppContext) {
    assert_eq!(keys(cx, "cmd-k cmd-up cmd-enter"), b"");
}

#[gpui::test]
fn shift_page_keys_scroll_the_history_instead_of_being_sent(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    let lines: String = (0..60).map(|i| format!("line {i}\r\n")).collect();
    h.output(&lines);
    h.window.simulate_keystrokes("shift-pageup");
    h.frame();
    let offset = |h: &mut super::Harness| {
        h.terminal
            .read_with(&mut *h.window, |t, _| t.snapshot().display_offset)
    };
    assert_eq!(offset(&mut h), 10, "one screen (10 rows) up");
    h.window.simulate_keystrokes("shift-pagedown");
    assert_eq!(offset(&mut h), 0);
    h.window.simulate_keystrokes("shift-home");
    assert!(offset(&mut h) > 10, "to the oldest line");
    h.window.simulate_keystrokes("shift-end");
    assert_eq!(offset(&mut h), 0);
    assert_eq!(h.written(), b"", "nothing reached the process");
    // On the alternate screen there is no history: the keys are the application's.
    h.output("\x1b[?1049h");
    h.window.simulate_keystrokes("shift-pageup");
    assert_eq!(h.written(), b"\x1b[5;2~");
}

#[gpui::test]
fn typing_returns_to_the_live_screen(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    let lines: String = (0..60).map(|i| format!("line {i}\r\n")).collect();
    h.output(&lines);
    h.terminal.update(&mut *h.window, |terminal, cx| {
        terminal.scroll(oxikube_terminal::TerminalScroll::Lines(5), cx)
    });
    assert!(
        h.terminal
            .read_with(&mut *h.window, |t, _| t.is_scrolled_back())
    );
    h.window.simulate_input("x");
    assert!(
        !h.terminal
            .read_with(&mut *h.window, |t, _| t.is_scrolled_back())
    );
}

#[gpui::test]
fn an_unfocused_terminal_receives_no_keys(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.window.update(|window, cx| window.blur(cx));
    h.frame();
    h.window.simulate_keystrokes("up enter");
    h.window.simulate_input("abc");
    assert_eq!(h.written(), b"");
}

#[gpui::test]
fn an_echo_is_painted_within_one_frame_of_the_key(cx: &mut TestAppContext) {
    // Input to pixel (docs/PERFORMANCE.md: <= 1 frame): the key reaches the backend, the echo comes
    // back through the pump, and the frame after one frame interval has it painted. Nothing in
    // the mapping layer waits.
    let backend = oxikube_testkit::fakes::FakeTerminalBackend::echo();
    let mut h = super::harness_on(cx, 480., 130., FONT_SIZE, None, backend);
    h.window.simulate_input("k");
    h.window.run_until_parked();
    h.window
        .executor()
        .advance_clock(oxikube_runtime::FRAME_INTERVAL);
    h.window.run_until_parked();
    h.window.draw_frame();
    let row = h
        .terminal
        .read_with(&mut *h.window, |t, _| t.snapshot().row_text(0));
    assert_eq!(
        row, "k",
        "the echo is in the frame one interval after the key"
    );
    // A mapped key takes the same path: Enter's carriage return comes back and moves the cursor.
    h.window.simulate_keystrokes("enter");
    h.frame();
    let column = h
        .terminal
        .read_with(&mut *h.window, |t, _| t.snapshot().cursor.column);
    assert_eq!(column, 0);
}

/// A key-down as Windows reports AltGr+Q on a German layout: Ctrl+Alt held, `key` the physical
/// key, `key_char` the typed character, and the platform asking for text input.
fn altgr_at(prefer_character_input: bool) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke {
            modifiers: Modifiers {
                control: true,
                alt: true,
                ..Modifiers::default()
            },
            key: "q".into(),
            key_char: Some("@".into()),
        },
        is_held: false,
        prefer_character_input,
    }
}

#[gpui::test]
fn altgr_characters_are_left_to_the_input_handler(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.window.simulate_event(altgr_at(true));
    // Not mapped to ESC + Ctrl-Q: the character belongs to the platform's text input.
    assert_eq!(h.written(), b"");
    // The same chord without the flag is still the Ctrl+Alt control code.
    h.window.simulate_event(altgr_at(false));
    assert_eq!(h.written(), b"\x1b\x11");
}
