//! Find in scrollback (E09-S11): the bar, typing a pattern, the matches, stepping through them,
//! output arriving meanwhile, an invalid pattern, closing.

use gpui::{Entity, TestAppContext};
use oxikube_terminal::view::{BackendDescriptor, REFRESH_DELAY, TerminalView};

use super::*;

fn open_terminal(
    h: &mut Harness,
) -> (
    Entity<TerminalView>,
    oxikube_testkit::fakes::FakeTerminalBackend,
) {
    let view = h.open(BackendDescriptor::local(None));
    h.frame();
    let backend = h.backend(0);
    (view, backend)
}

fn open_search(h: &mut Harness, view: &Entity<TerminalView>) {
    h.vcx
        .update(|window, cx| view.update(cx, |view, cx| view.open_search(window, cx)));
    h.frame();
}

/// Types into the focused field, then lets the scan finish.
fn type_pattern(h: &mut Harness, text: &str) {
    h.vcx.simulate_input(text);
    h.frame();
}

fn status(h: &mut Harness, view: &Entity<TerminalView>) -> (bool, usize, Option<usize>) {
    h.vcx.update(|_, cx| {
        let view = view.read(cx);
        (
            view.search_open(),
            view.search_matches().len(),
            view.search_current(),
        )
    })
}

fn offset(h: &mut Harness, view: &Entity<TerminalView>) -> usize {
    h.vcx.update(|_, cx| {
        let state = view.read(cx).terminal().expect("running").clone();
        state.read(cx).snapshot().display_offset
    })
}

fn lines(backend: &oxikube_testkit::fakes::FakeTerminalBackend, count: usize) {
    for line in 0..count {
        let text = if line % 10 == 3 {
            format!("line {line} needle\r\n")
        } else {
            format!("line {line}\r\n")
        };
        backend.output(text);
    }
}

#[gpui::test]
fn search_opens_a_bar_and_finds_matches_in_the_scrollback(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (view, backend) = open_terminal(&mut h);
    lines(&backend, 60);
    h.frame();
    assert!(!h.drawn("terminal-search"), "no bar until asked");

    open_search(&mut h, &view);
    assert!(h.drawn("terminal-search"), "the bar is shown");
    assert!(h.drawn("terminal-search-input"));
    assert_eq!(status(&mut h, &view), (true, 0, None));

    type_pattern(&mut h, "needle");
    // Lines 3, 13, ... 53: six matches, the last (nearest the bottom) is current.
    assert_eq!(status(&mut h, &view), (true, 6, Some(5)));
    assert_eq!(
        h.vcx
            .update(|_, cx| view.read(cx).search_pattern().to_owned()),
        "needle"
    );
    assert!(h.drawn("terminal-search-status"));
}

#[gpui::test]
fn next_and_previous_wrap_and_scroll_the_history_to_the_match(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (view, backend) = open_terminal(&mut h);
    // The window is tall: enough lines that the early ones are in the history.
    lines(&backend, 300);
    h.frame();
    open_search(&mut h, &view);
    type_pattern(&mut h, "needle");
    assert_eq!(status(&mut h, &view).2, Some(29));

    h.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.search_next(cx)));
    assert_eq!(status(&mut h, &view).2, Some(0), "wraps past the end");
    let at_first = offset(&mut h, &view);
    assert!(at_first > 200, "scrolled up to line 3 of 300: {at_first}");

    h.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.search_previous(cx)));
    assert_eq!(status(&mut h, &view).2, Some(29), "wraps before the start");
    assert!(
        offset(&mut h, &view) < at_first,
        "back down near the live screen"
    );

    h.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.search_previous(cx)));
    assert_eq!(status(&mut h, &view).2, Some(28));
}

#[gpui::test]
fn the_commands_drive_the_same_operations(cx: &mut TestAppContext) {
    // Enter in the field, the arrows and the keys all send `terminal::Search*` through the
    // services' dispatcher; with the recorder installed the test sees them instead.
    let mut h = harness(cx);
    let (view, _backend) = open_terminal(&mut h);
    open_search(&mut h, &view);
    h.vcx.simulate_keystrokes("a enter");
    h.vcx.simulate_keystrokes("shift-enter");
    assert_eq!(
        *h.recorder.0.borrow(),
        [Command::TerminalSearchNext, Command::TerminalSearchPrevious]
    );
}

#[gpui::test]
fn an_invalid_pattern_says_so_and_keeps_the_last_matches(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (view, backend) = open_terminal(&mut h);
    lines(&backend, 30);
    h.frame();
    open_search(&mut h, &view);
    type_pattern(&mut h, "needle");
    let good = status(&mut h, &view);
    assert_eq!(good.1, 3);

    // `needle(` is not a regular expression.
    type_pattern(&mut h, "(");
    assert_eq!(status(&mut h, &view), good, "the last good matches stay");
    let error = h
        .vcx
        .update(|_, cx| view.read(cx).search_error().map(str::to_owned));
    assert_eq!(error.as_deref(), Some("Invalid regular expression"));
    assert!(
        !error.unwrap().contains("needle"),
        "the message never repeats the pattern"
    );
    // Fixing the pattern clears the complaint.
    h.vcx.simulate_keystrokes("backspace");
    h.frame();
    assert_eq!(status(&mut h, &view), good);
    assert_eq!(
        h.vcx.update(|_, cx| view.read(cx).search_error().is_none()),
        true
    );
}

#[gpui::test]
fn output_while_the_bar_is_open_updates_the_matches_soon(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (view, backend) = open_terminal(&mut h);
    lines(&backend, 30);
    h.frame();
    open_search(&mut h, &view);
    type_pattern(&mut h, "needle");
    assert_eq!(status(&mut h, &view).1, 3);

    backend.output("one more needle\r\n");
    h.frame();
    assert_eq!(status(&mut h, &view).1, 3, "not looked up yet");
    h.vcx.executor().advance_clock(REFRESH_DELAY);
    h.frame();
    assert_eq!(status(&mut h, &view).1, 4, "found after the short delay");
}

#[gpui::test]
fn closing_hides_the_bar_forgets_the_matches_and_returns_the_keyboard(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (view, backend) = open_terminal(&mut h);
    lines(&backend, 30);
    h.frame();
    open_search(&mut h, &view);
    type_pattern(&mut h, "needle");
    assert_eq!(status(&mut h, &view).1, 3);

    h.vcx
        .update(|window, cx| view.update(cx, |view, cx| view.close_search(window, cx)));
    h.frame();
    assert_eq!(status(&mut h, &view), (false, 0, None));
    assert!(!h.drawn("terminal-search"));
    // Typing goes to the process again.
    h.vcx.simulate_input("x");
    assert!(
        backend.written().ends_with(b"x"),
        "keys reach the process again"
    );
}

#[gpui::test]
fn an_empty_pattern_matches_nothing(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (view, backend) = open_terminal(&mut h);
    lines(&backend, 30);
    h.frame();
    open_search(&mut h, &view);
    type_pattern(&mut h, "n");
    assert!(status(&mut h, &view).1 > 0);
    h.vcx.simulate_keystrokes("backspace");
    h.frame();
    assert_eq!(status(&mut h, &view), (true, 0, None));
}

#[gpui::test]
fn the_search_is_never_saved_with_the_tab(cx: &mut TestAppContext) {
    use oxikube_workspace::Item as _;
    let mut h = harness(cx);
    let (view, backend) = open_terminal(&mut h);
    lines(&backend, 30);
    h.frame();
    open_search(&mut h, &view);
    type_pattern(&mut h, "needle");
    let saved = h
        .vcx
        .update(|_, cx| view.read(cx).serialize(cx))
        .expect("a saved state")
        .to_string();
    assert!(!saved.contains("needle"), "{saved}");
}

#[gpui::test]
fn reopening_the_bar_searches_the_kept_pattern_again(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (view, backend) = open_terminal(&mut h);
    lines(&backend, 30);
    h.frame();
    open_search(&mut h, &view);
    type_pattern(&mut h, "needle");
    assert_eq!(status(&mut h, &view).1, 3);

    h.vcx
        .update(|window, cx| view.update(cx, |view, cx| view.close_search(window, cx)));
    h.frame();
    assert_eq!(status(&mut h, &view), (false, 0, None));

    open_search(&mut h, &view);
    assert_eq!(
        h.vcx
            .update(|_, cx| view.read(cx).search_pattern().to_owned()),
        "needle",
        "the field still shows the pattern"
    );
    assert_eq!(
        status(&mut h, &view),
        (true, 3, Some(2)),
        "and it was searched again, not left at no matches"
    );
    // Enter steps without editing the text first.
    h.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.search_next(cx)));
    assert_eq!(status(&mut h, &view).2, Some(0));
}

#[gpui::test]
fn reopening_the_bar_shows_an_invalid_pattern_s_error_again(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (view, _backend) = open_terminal(&mut h);
    open_search(&mut h, &view);
    type_pattern(&mut h, "(");
    assert!(h.vcx.update(|_, cx| view.read(cx).search_error().is_some()));

    h.vcx
        .update(|window, cx| view.update(cx, |view, cx| view.close_search(window, cx)));
    h.frame();
    open_search(&mut h, &view);
    assert_eq!(
        h.vcx
            .update(|_, cx| view.read(cx).search_error().map(str::to_owned))
            .as_deref(),
        Some("Invalid regular expression")
    );
}

#[gpui::test]
fn clearing_the_terminal_with_the_bar_open_drops_the_stale_matches(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (view, backend) = open_terminal(&mut h);
    lines(&backend, 300);
    h.frame();
    open_search(&mut h, &view);
    type_pattern(&mut h, "needle");
    assert_eq!(status(&mut h, &view).1, 30);

    // Nothing is printed: only the grid changes under the matches.
    let state = h
        .vcx
        .update(|_, cx| view.read(cx).terminal().expect("running").clone());
    h.vcx
        .update(|_, cx| state.update(cx, |state, cx| state.clear(cx)));
    h.frame();
    h.vcx.executor().advance_clock(REFRESH_DELAY);
    h.frame();
    let after = status(&mut h, &view);
    let expected = h.vcx.update(|_, cx| {
        let snapshot = state.read(cx).snapshot();
        (0..snapshot.rows)
            .filter(|row| snapshot.row_text(*row).contains("needle"))
            .count()
    });
    assert!(after.1 < 30, "the history's matches are gone: {after:?}");
    assert_eq!(after.1, expected, "only what is still on the grid matches");
}

#[gpui::test]
fn output_scrolling_the_screen_keeps_the_current_match(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (view, backend) = open_terminal(&mut h);
    lines(&backend, 300);
    h.frame();
    open_search(&mut h, &view);
    type_pattern(&mut h, "needle");
    h.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.search_next(cx)));
    assert_eq!(status(&mut h, &view), (true, 30, Some(0)));
    let first = h
        .vcx
        .update(|_, cx| view.read(cx).search_matches()[0].start);

    // Plain output moves every line (and so every match) up the grid; the user stays on the
    // match they were on, which is now 25 lines higher.
    for _ in 0..25 {
        backend.output("filler\r\n");
    }
    h.frame();
    h.vcx.executor().advance_clock(REFRESH_DELAY);
    h.frame();
    assert_eq!(status(&mut h, &view), (true, 30, Some(0)));
    let moved = h
        .vcx
        .update(|_, cx| view.read(cx).search_matches()[0].start);
    assert_eq!(moved.line, first.line - 25, "the match moved with its line");
    assert_eq!(moved.column, first.column);
}
