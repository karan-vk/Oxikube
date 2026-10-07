//! Title, bell, errors, exit and kill reach the view as events; search and selection work
//! through the entity.

use gpui::TestAppContext;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::ExitStatus;
use oxikube_terminal::{GridPoint, SelectionKind, SelectionSide, TerminalEvent};
use oxikube_testkit::fakes::FakeTerminalBackend;

use super::{harness, next_frame};

#[gpui::test]
fn title_and_bell_are_events(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (40, 5));
    backend.output("\x1b]2;k9s\x07\x07");
    next_frame(cx);
    let events = h.events.borrow();
    assert!(matches!(&events[0], TerminalEvent::TitleChanged(Some(title)) if &**title == "k9s"));
    assert!(matches!(events[1], TerminalEvent::Bell));
    drop(events);
    let title = h.terminal.read_with(cx, |terminal, _| terminal.title());
    assert_eq!(title.as_deref(), Some("k9s"));
}

#[gpui::test]
fn exit_is_reported_and_the_grid_kept(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (40, 5));
    backend.output("bye");
    backend.exit(ExitStatus::with_code(3));
    next_frame(cx);
    let status = h
        .terminal
        .read_with(cx, |terminal, _| terminal.exit_status().cloned());
    assert_eq!(status, Some(ExitStatus::with_code(3)));
    assert!(
        matches!(h.events.borrow().last(), Some(TerminalEvent::Exited(s)) if s.code == Some(3))
    );
    assert_eq!(h.row(cx, 0), "bye");
}

#[gpui::test]
fn errors_are_reported(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (40, 5));
    backend.error(OxiError::network("connection reset"));
    next_frame(cx);
    let kind = h
        .terminal
        .read_with(cx, |terminal, _| terminal.last_error().map(OxiError::kind));
    assert_eq!(kind, Some(ErrorKind::Network));
    assert!(matches!(
        h.events.borrow().last(),
        Some(TerminalEvent::Error(_))
    ));
}

#[gpui::test]
fn kill_ends_the_session(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (40, 5));
    let kill = h.terminal.update(cx, |terminal, cx| terminal.kill(cx));
    cx.run_until_parked();
    assert!(cx.foreground_executor().block_test(kill).unwrap().is_ok());
    next_frame(cx);
    assert_eq!(backend.kill_count(), 1);
    let status = h
        .terminal
        .read_with(cx, |terminal, _| terminal.exit_status().cloned());
    assert_eq!(status, Some(ExitStatus::killed_by("KILL")));
}

#[gpui::test]
fn search_runs_off_the_ui_thread_and_selection_copies(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (20, 3));
    backend.output("pod-a Running\r\npod-b Failed\r\npod-c Running\r\nprompt$ ");
    next_frame(cx);
    let task = h
        .terminal
        .update(cx, |terminal, cx| terminal.search("Running", cx));
    cx.run_until_parked();
    let matches = cx.foreground_executor().block_test(task).unwrap();
    assert_eq!(matches.len(), 2);
    // The first match scrolled into the history (4 lines on a 3-row screen).
    assert_eq!(matches[0].start, GridPoint::new(-1, 6));

    h.terminal.update(cx, |terminal, cx| {
        terminal.scroll_to(matches[0].start, cx);
        terminal.start_selection(
            SelectionKind::Word,
            matches[0].start,
            SelectionSide::Left,
            cx,
        );
    });
    let (text, offset) = h.terminal.read_with(cx, |terminal, _| {
        (
            terminal.selection_text(),
            terminal.snapshot().display_offset,
        )
    });
    assert_eq!(text.as_deref(), Some("Running"));
    assert_eq!(offset, 1);
    h.terminal
        .update(cx, |terminal, cx| terminal.clear_selection(cx));
    assert_eq!(h.terminal.read_with(cx, |t, _| t.selection_text()), None);
}

#[gpui::test]
fn a_search_longer_than_one_slice_finds_everything(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (20, 3));
    let lines = 2 * oxikube_terminal::grid::SEARCH_SLICE_LINES + 7;
    let output: String = (0..lines)
        .map(|line| format!("pod-{line} Running\r\n"))
        .collect();
    backend.output(output);
    next_frame(cx);
    let task = h
        .terminal
        .update(cx, |terminal, cx| terminal.search("Running", cx));
    cx.run_until_parked();
    let matches = cx.foreground_executor().block_test(task).unwrap();
    assert_eq!(matches.len(), lines);
    assert!(matches.windows(2).all(|pair| pair[0].start < pair[1].start));

    // The UI side holds no lock afterwards and output flows again.
    backend.output("pod-last Running");
    next_frame(cx);
    let row = h
        .terminal
        .read_with(cx, |terminal, _| terminal.snapshot().row_text(2));
    assert_eq!(row, "pod-last Running");
}

#[gpui::test]
fn input_stops_when_the_session_ends_or_its_input_is_closed(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (40, 5));
    h.terminal.update(cx, |terminal, _| terminal.input("ls\r"));
    next_frame(cx);
    assert_eq!(backend.written(), b"ls\r", "a running session takes input");

    // A dropped connection: the view closes the input, the screen stays.
    backend.output("kept");
    next_frame(cx);
    h.terminal.update(cx, |terminal, _| terminal.close_input());
    assert!(
        !h.terminal
            .read_with(cx, |terminal, _| terminal.accepts_input())
    );
    h.terminal
        .update(cx, |terminal, _| terminal.input("rm -rf /\r"));
    next_frame(cx);
    assert_eq!(
        backend.written(),
        b"ls\r",
        "nothing more reaches the process"
    );
    assert_eq!(h.row(cx, 0), "kept");
}

#[gpui::test]
fn an_ended_session_takes_no_input(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (40, 5));
    backend.exit(ExitStatus::with_code(0));
    next_frame(cx);
    assert!(
        !h.terminal
            .read_with(cx, |terminal, _| terminal.accepts_input())
    );
    h.terminal.update(cx, |terminal, _| terminal.input("x"));
    next_frame(cx);
    assert!(backend.written().is_empty());
}
