//! Copy, copy on select, paste (bracketed, multi-line confirmation).

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, ClipboardItem, KeyBinding, Modifiers, MouseButton, TestAppContext, Window, point, px,
};
use oxikube_terminal::input::{Copy, PasteConfirm};

use super::{FONT_SIZE, Harness, harness, harness_with};
use crate::configure;

/// Records confirmation requests and lets the test accept them.
#[derive(Default)]
struct Confirmations {
    asked: RefCell<Vec<String>>,
    accept: RefCell<Vec<Rc<dyn Fn(&mut Window, &mut App)>>>,
}

impl PasteConfirm for Confirmations {
    fn confirm(
        &self,
        text: &str,
        accept: Rc<dyn Fn(&mut Window, &mut App)>,
        _: &mut Window,
        _: &mut App,
    ) {
        self.asked.borrow_mut().push(text.to_owned());
        self.accept.borrow_mut().push(accept);
    }
}

fn bind(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("cmd-c", Copy, Some("Terminal")),
            KeyBinding::new("cmd-v", oxikube_terminal::input::Paste, Some("Terminal")),
        ]);
    });
}

fn clipboard(cx: &mut TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

fn set_clipboard(cx: &mut TestAppContext, text: &str) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
}

/// Selects "hello" on the first row with the mouse.
fn select_hello(h: &mut Harness) {
    h.output("hello world");
    let start = Harness::at(0, 0) - point(px(2.), px(0.));
    h.window
        .simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_move(Harness::at(0, 4), MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_up(Harness::at(0, 4), MouseButton::Left, Modifiers::none());
}

#[gpui::test]
fn copy_puts_the_selection_on_the_clipboard(cx: &mut TestAppContext) {
    bind(cx);
    set_clipboard(cx, "before");
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    select_hello(&mut h);
    assert_eq!(
        clipboard(cx).as_deref(),
        Some("before"),
        "copy on select is off"
    );
    h.window.simulate_keystrokes("cmd-c");
    assert_eq!(clipboard(cx).as_deref(), Some("hello"));
    assert_eq!(h.written(), b"", "cmd-c is not terminal input");
}

#[gpui::test]
fn copy_without_a_selection_changes_nothing(cx: &mut TestAppContext) {
    bind(cx);
    set_clipboard(cx, "keep me");
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("hello");
    h.window.simulate_keystrokes("cmd-c");
    assert_eq!(clipboard(cx).as_deref(), Some("keep me"));
    // ctrl-c is the interrupt, not a copy, with or without a selection.
    h.window.simulate_keystrokes("ctrl-c");
    assert_eq!(h.written(), b"\x03");
    assert_eq!(clipboard(cx).as_deref(), Some("keep me"));
}

#[gpui::test]
fn the_copy_action_is_what_the_palette_dispatches(cx: &mut TestAppContext) {
    set_clipboard(cx, "before");
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    select_hello(&mut h);
    h.window.dispatch_action(Copy);
    assert_eq!(clipboard(cx).as_deref(), Some("hello"));
}

#[gpui::test]
fn copy_on_select_copies_when_the_drag_ends(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "copy_on_select": true } }"#);
    set_clipboard(cx, "before");
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("hello world");
    let start = Harness::at(0, 0) - point(px(2.), px(0.));
    h.window
        .simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_move(Harness::at(0, 4), MouseButton::Left, Modifiers::none());
    assert_eq!(
        clipboard(cx).as_deref(),
        Some("before"),
        "not while dragging"
    );
    h.window
        .simulate_mouse_up(Harness::at(0, 4), MouseButton::Left, Modifiers::none());
    assert_eq!(clipboard(cx).as_deref(), Some("hello"));
}

#[gpui::test]
fn a_plain_click_copies_nothing_even_with_copy_on_select(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "copy_on_select": true } }"#);
    set_clipboard(cx, "before");
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("hello world");
    h.window
        .simulate_mouse_down(Harness::at(0, 2), MouseButton::Left, Modifiers::none());
    h.window
        .simulate_mouse_up(Harness::at(0, 2), MouseButton::Left, Modifiers::none());
    assert_eq!(clipboard(cx).as_deref(), Some("before"));
}

#[gpui::test]
fn paste_sends_the_clipboard_to_the_process(cx: &mut TestAppContext) {
    bind(cx);
    set_clipboard(cx, "echo hi");
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.window.simulate_keystrokes("cmd-v");
    assert_eq!(
        h.written(),
        b"echo hi",
        "no brackets: the process did not ask"
    );
}

#[gpui::test]
fn paste_is_bracketed_when_the_process_asked(cx: &mut TestAppContext) {
    configure(
        cx,
        r#"{ "terminal": { "confirm_multiline_paste": false } }"#,
    );
    set_clipboard(cx, "a\nb");
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("\x1b[?2004h");
    h.window.dispatch_action(oxikube_terminal::input::Paste);
    assert_eq!(h.written(), b"\x1b[200~a\nb\x1b[201~");
}

#[gpui::test]
fn an_embedded_end_marker_is_stripped_from_a_bracketed_paste(cx: &mut TestAppContext) {
    set_clipboard(cx, "ls\x1b[201~; rm -rf ~");
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("\x1b[?2004h");
    h.window.dispatch_action(oxikube_terminal::input::Paste);
    assert_eq!(h.written(), b"\x1b[200~ls; rm -rf ~\x1b[201~");
}

#[gpui::test]
fn without_bracketed_paste_newlines_are_returns(cx: &mut TestAppContext) {
    configure(
        cx,
        r#"{ "terminal": { "confirm_multiline_paste": false } }"#,
    );
    set_clipboard(cx, "a\nb\n");
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.window.dispatch_action(oxikube_terminal::input::Paste);
    assert_eq!(h.written(), b"a\rb\r");
}

#[gpui::test]
fn a_multi_line_paste_waits_for_confirmation(cx: &mut TestAppContext) {
    let confirm = Rc::new(Confirmations::default());
    set_clipboard(cx, "line one\nline two");
    let mut h = harness_with(cx, 480., 130., FONT_SIZE, Some(confirm.clone()));
    h.window.dispatch_action(oxikube_terminal::input::Paste);
    assert_eq!(*confirm.asked.borrow(), ["line one\nline two"]);
    assert_eq!(h.written(), b"", "nothing is sent before the user confirms");

    let accept = confirm.accept.borrow()[0].clone();
    h.window.update(|window, cx| accept(window, cx));
    assert_eq!(h.written(), b"line one\rline two");
}

#[gpui::test]
fn cancelling_the_confirmation_sends_nothing(cx: &mut TestAppContext) {
    let confirm = Rc::new(Confirmations::default());
    set_clipboard(cx, "rm -rf build\n");
    let mut h = harness_with(cx, 480., 130., FONT_SIZE, Some(confirm.clone()));
    h.window.dispatch_action(oxikube_terminal::input::Paste);
    assert_eq!(confirm.asked.borrow().len(), 1, "a trailing newline counts");
    // The user cancels: the accept hook is simply never run.
    drop(confirm.accept.borrow_mut().drain(..));
    assert_eq!(h.written(), b"");
}

#[gpui::test]
fn single_line_pastes_never_ask(cx: &mut TestAppContext) {
    let confirm = Rc::new(Confirmations::default());
    set_clipboard(cx, "kubectl get pods -A");
    let mut h = harness_with(cx, 480., 130., FONT_SIZE, Some(confirm.clone()));
    h.window.dispatch_action(oxikube_terminal::input::Paste);
    assert!(confirm.asked.borrow().is_empty());
    assert_eq!(h.written(), b"kubectl get pods -A");
}

#[gpui::test]
fn the_setting_turns_the_confirmation_off(cx: &mut TestAppContext) {
    configure(
        cx,
        r#"{ "terminal": { "confirm_multiline_paste": false } }"#,
    );
    let confirm = Rc::new(Confirmations::default());
    set_clipboard(cx, "a\nb");
    let mut h = harness_with(cx, 480., 130., FONT_SIZE, Some(confirm.clone()));
    h.window.dispatch_action(oxikube_terminal::input::Paste);
    assert!(confirm.asked.borrow().is_empty());
    assert_eq!(h.written(), b"a\rb");
    // And on again, live.
    configure(cx, r#"{ "terminal": { "confirm_multiline_paste": true } }"#);
    h.window.dispatch_action(oxikube_terminal::input::Paste);
    assert_eq!(confirm.asked.borrow().len(), 1);
}

#[gpui::test]
fn pasting_scrolls_back_to_the_live_screen(cx: &mut TestAppContext) {
    set_clipboard(cx, "x");
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    let lines: String = (0..60).map(|i| format!("line {i}\r\n")).collect();
    h.output(&lines);
    h.terminal.update(&mut *h.window, |terminal, cx| {
        terminal.scroll(oxikube_terminal::TerminalScroll::Lines(5), cx)
    });
    h.window.dispatch_action(oxikube_terminal::input::Paste);
    assert!(
        !h.terminal
            .read_with(&mut *h.window, |t, _| t.is_scrolled_back())
    );
}

#[gpui::test]
fn an_empty_clipboard_pastes_nothing(cx: &mut TestAppContext) {
    cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.window.dispatch_action(oxikube_terminal::input::Paste);
    assert_eq!(h.written(), b"");
}
