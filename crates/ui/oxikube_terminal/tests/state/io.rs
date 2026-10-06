//! The writer: input, emulator replies and resizes reach the backend in order.

use gpui::TestAppContext;
use oxikube_ports::TerminalSize;
use oxikube_terminal::TerminalEvent;
use oxikube_terminal::grid::TermRgb;
use oxikube_testkit::fakes::FakeTerminalBackend;

use super::{harness, next_frame};

#[gpui::test]
fn input_is_written_in_order(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::echo();
    let h = harness(cx, &backend, (40, 5));
    h.terminal.read_with(cx, |terminal, _| {
        terminal.input("echo ");
        terminal.input(b"hi\r".to_vec());
    });
    next_frame(cx);
    assert_eq!(backend.written(), b"echo hi\r");
    // The echo came back through the pump.
    assert_eq!(h.row(cx, 0), "echo hi");
}

#[gpui::test]
fn emulator_replies_go_back_to_the_process(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let _h = harness(cx, &backend, (40, 5));
    backend.output("ab\x1b[6n");
    next_frame(cx);
    assert_eq!(backend.written(), b"\x1b[1;3R");
}

#[gpui::test]
fn colour_queries_are_answered_by_the_view(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (40, 5));
    backend.output("\x1b]11;?\x07");
    next_frame(cx);
    let request = h
        .events
        .borrow()
        .iter()
        .find_map(|event| match event {
            TerminalEvent::ColorRequest(request) => Some(request.clone()),
            _ => None,
        })
        .expect("a colour request");
    assert_eq!(request.index(), 257);
    h.terminal.read_with(cx, |terminal, _| {
        terminal.reply_color(&request, TermRgb { r: 0, g: 0, b: 0 })
    });
    next_frame(cx);
    assert_eq!(backend.written(), b"\x1b]11;rgb:0000/0000/0000\x07");
}

#[gpui::test]
fn resize_applies_at_once_and_coalesces_for_the_backend(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (80, 24));
    backend.output("0123456789abcdefghij");
    next_frame(cx);

    // A drag: three sizes inside one frame.
    h.terminal.update(cx, |terminal, cx| {
        terminal.resize(TerminalSize::new(120, 40), cx);
        terminal.resize(TerminalSize::new(100, 30), cx);
        terminal.resize(TerminalSize::new(10, 20).with_pixels(80, 320), cx);
    });
    // The grid reflowed before any task ran: the next frame paints the new size.
    let snapshot = h.terminal.read_with(cx, |terminal, _| terminal.snapshot());
    assert_eq!((snapshot.columns, snapshot.rows), (10, 20));
    // The 20-character line wrapped at 10 columns; its first half went into the history.
    assert_eq!(snapshot.row_text(0), "abcdefghij");
    assert_eq!(snapshot.history_size, 1);

    next_frame(cx);
    assert_eq!(
        backend.resizes(),
        [TerminalSize::new(10, 20).with_pixels(80, 320)],
        "only the last size of the drag reaches the backend"
    );

    // The same size again is not sent; the initial size never is.
    h.terminal.update(cx, |terminal, cx| {
        terminal.resize(TerminalSize::new(10, 20).with_pixels(80, 320), cx)
    });
    next_frame(cx);
    assert_eq!(backend.resizes().len(), 1);
}
