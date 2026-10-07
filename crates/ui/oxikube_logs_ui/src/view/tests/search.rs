//! Search and filter in the view (E08-S03): the `/` bar over the stream, highlights, the count,
//! next / previous, filter mode, inverse, an invalid pattern, the per-session memory.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::log::LogLine;
use oxikube_testkit::Timeline;

use super::fixture::{Fx, line, lines, pod_ref};
use crate::search::SearchMode;

/// `n` lines, then one more every second: `line(i)` is an error when `i % 5 == 4`.
fn trickle(first: usize, more: usize) -> Timeline<LogLine> {
    let mut timeline = Timeline::new();
    for i in 0..first {
        timeline = timeline.ok_at(Duration::ZERO, line(i));
    }
    for i in 0..more {
        timeline = timeline.ok_at(Duration::from_secs(i as u64 + 1), line(first + i));
    }
    timeline.keep_open()
}

fn open(fx: &mut Fx, n: usize) -> gpui::Entity<crate::LogView> {
    let view = fx.open(Timeline::immediate(lines(0, n)).keep_open());
    fx.draw();
    view
}

/// Opens the bar with `/` and types `text`.
fn search(fx: &mut Fx, text: &str) {
    fx.keys("/");
    let typed = text
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    fx.vcx.simulate_keystrokes(&typed);
    fx.settle();
}

fn tick(fx: &mut Fx, seconds: u64) {
    for _ in 0..seconds {
        fx.ports.logs.clock().advance(Duration::from_secs(1));
        fx.settle();
    }
}

fn status(fx: &mut Fx, view: &gpui::Entity<crate::LogView>) -> String {
    fx.read(view, |v| v.search_state().status_for(v.search_counts()))
}

#[gpui::test]
fn slash_opens_the_bar_through_its_command_and_typing_highlights_the_matches(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 20);
    assert!(!fx.drawn("log-search"), "the bar is closed until asked for");

    fx.keys("/");
    assert_eq!(
        fx.dispatcher.sent(),
        [Command::LogsFind {
            target: pod_ref(),
            pattern: None
        }],
        "the key is the command"
    );
    assert!(fx.drawn("log-search") && fx.drawn("log-search-input"));
    assert!(
        fx.read(&view, |v| v.is_searching()),
        "the field has the focus"
    );

    // Bare keys are text while the field is focused: `s` does not toggle autoscroll.
    fx.vcx.simulate_keystrokes("e r r o r");
    fx.settle();
    assert!(fx.read(&view, |v| v.autoscroll()));
    assert_eq!(
        fx.read(&view, |v| v.search_state().text().to_owned()),
        "error"
    );
    // Lines 4, 9, 14 and 19 are errors; case-insensitively `error` finds `ERROR`.
    let counts = fx.read(&view, |v| v.search_counts());
    assert_eq!((counts.matches, counts.lines), (4, 20));
    assert_eq!(status(&mut fx, &view), "4 matches");
    assert!(fx.drawn("log-search-status"));
    // The matching rows carry a highlight over the word; the others none.
    let highlights = fx.read(&view, |v| {
        (0..20).map(|i| v.row_highlights(i)).collect::<Vec<_>>()
    });
    for (i, spans) in highlights.iter().enumerate() {
        if i % 5 == 4 {
            assert_eq!(spans, &[0..5], "row {i}: ERROR");
        } else {
            assert!(spans.is_empty(), "row {i}");
        }
    }
    // Every line is still a row: highlight mode hides nothing.
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 20);
}

#[gpui::test]
fn the_case_toggle_and_the_regex_change_what_matches(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 20);
    search(&mut fx, "error");
    assert_eq!(fx.read(&view, |v| v.search_counts().matches), 4);

    fx.click("log-search-case");
    assert!(fx.read(&view, |v| v.search_state().case_sensitive()));
    assert_eq!(
        fx.read(&view, |v| v.search_counts().matches),
        0,
        "`error` no longer finds `ERROR`"
    );
    assert_eq!(status(&mut fx, &view), "No matches");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsToggleCase { target: pod_ref() }),
        "the button sends the command"
    );
    fx.click("log-search-case");
    assert_eq!(fx.read(&view, |v| v.search_counts().matches), 4);

    // A regex: lines 1 and 11 (`line 1` then end, or `line 1x`): `line 1\d?$`.
    fx.vcx
        .update(|window, cx| view.update(cx, |v, cx| v.set_search_text(r"line 1\d?$", window, cx)));
    fx.settle();
    assert_eq!(fx.read(&view, |v| v.search_counts().matches), 11);
}

#[gpui::test]
fn enter_and_shift_enter_walk_the_matches_with_a_count_and_wrap(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 100);
    search(&mut fx, "error"); // lines 4, 9, ..., 99: twenty of them
    assert_eq!(fx.read(&view, |v| v.search_counts().matches), 20);
    // "Next" starts from the top of the screen: look at the start of the log first.
    fx.wheel(500.);
    assert_eq!(fx.read(&view, |v| v.top_seq()), Some(0));

    fx.keys("enter");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsNextMatch { target: pod_ref() })
    );
    assert!(
        !fx.read(&view, |v| v.autoscroll()),
        "a jump pauses autoscroll"
    );
    let first = fx.read(&view, |v| v.search_state().current());
    assert!(first.is_some());
    assert_eq!(status(&mut fx, &view), "1 of 20");
    // The match is on screen, a few rows below the top.
    fx.draw();
    let (top, built, row) = fx.read(&view, |v| {
        (
            v.top_row(),
            v.rows_built(),
            v.line_window().index_of(first.unwrap()).unwrap(),
        )
    });
    assert!(
        top <= row && row < top + built,
        "row {row} in {top}..{}",
        top + built
    );

    fx.keys("enter");
    assert_eq!(status(&mut fx, &view), "2 of 20");
    fx.keys("shift-enter");
    assert_eq!(status(&mut fx, &view), "1 of 20");
    // Backwards from the first wraps to the last, forwards from the last wraps to the first.
    fx.keys("shift-enter");
    assert_eq!(status(&mut fx, &view), "20 of 20");
    assert_eq!(fx.read(&view, |v| v.search_state().current()), Some(99));
    fx.keys("enter");
    assert_eq!(status(&mut fx, &view), "1 of 20");
    assert_eq!(
        fx.dispatcher
            .sent()
            .iter()
            .filter(|c| matches!(c, Command::LogsPreviousMatch { .. }))
            .count(),
        2
    );
}

#[gpui::test]
fn n_and_shift_n_step_outside_the_field_and_escape_closes_and_clears(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 100);
    search(&mut fx, "error");
    fx.wheel(500.);
    // Leave the field: focus the log itself, as a click on the rows does.
    fx.vcx.update(|window, cx| {
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
    });
    fx.settle();
    assert!(!fx.read(&view, |v| v.is_searching()));
    fx.keys("n");
    assert_eq!(status(&mut fx, &view), "1 of 20");
    fx.keys("n");
    assert_eq!(status(&mut fx, &view), "2 of 20");
    fx.keys("shift-n");
    assert_eq!(status(&mut fx, &view), "1 of 20");

    fx.keys("escape");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsCloseSearch { target: pod_ref() })
    );
    assert!(!fx.drawn("log-search"), "the bar is gone");
    let counts = fx.read(&view, |v| v.search_counts());
    assert_eq!(counts.matches, 0, "the highlights are cleared");
    assert!(fx.read(&view, |v| (0..100).all(|i| v.row_highlights(i).is_empty())));
    assert!(fx.read(&view, |v| v.search_state().matcher().is_none()));
}

#[gpui::test]
fn filter_mode_shows_only_the_matching_lines_and_inverse_the_others(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 20);
    search(&mut fx, "error");
    fx.click("log-search-filter");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsToggleFilterMode { target: pod_ref() })
    );
    assert_eq!(
        fx.read(&view, |v| v.search_state().mode()),
        SearchMode::Filter
    );
    let rows = fx.read(&view, |v| {
        (0..v.line_window().row_count())
            .filter_map(|i| v.row_text(i))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        rows,
        [
            "ERROR line 4",
            "ERROR line 9",
            "ERROR line 14",
            "ERROR line 19"
        ]
    );
    assert_eq!(status(&mut fx, &view), "4 of 20 lines");

    // Inverse keeps the 16 lines that are not errors.
    fx.click("log-search-inverse");
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 16);
    let rows = fx.read(&view, |v| {
        (0..v.line_window().row_count())
            .filter_map(|i| v.row_text(i))
            .collect::<Vec<_>>()
    });
    assert!(rows.iter().all(|r| !r.contains("ERROR")), "{rows:?}");

    // Back to highlighting: all 20 lines are rows again.
    fx.click("log-search-filter");
    fx.click("log-search-inverse");
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 20);
    assert_eq!(fx.read(&view, |v| v.search_counts().matches), 4);
}

#[gpui::test]
fn an_invalid_pattern_shows_why_and_keeps_the_last_good_one(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 20);
    search(&mut fx, "error");
    assert_eq!(fx.read(&view, |v| v.search_counts().matches), 4);
    // `error(` does not compile.
    fx.vcx.simulate_keystrokes("(");
    fx.settle();
    assert_eq!(
        fx.read(&view, |v| v.search_state().text().to_owned()),
        "error("
    );
    assert_eq!(status(&mut fx, &view), "Invalid pattern: unclosed group");
    assert_eq!(
        fx.read(&view, |v| v.search_counts().matches),
        4,
        "the last good pattern keeps its matches"
    );
    // Closing the group makes a valid pattern again (matching nothing here).
    fx.vcx.simulate_keystrokes(")");
    fx.settle();
    assert_eq!(fx.read(&view, |v| v.search_state().error().is_none()), true);
}

#[gpui::test]
fn lines_that_stream_in_are_matched_as_they_arrive_and_trimmed_ones_leave(cx: &mut TestAppContext) {
    // A ring of 100 lines: 80 arrive at once, then one per second.
    let mut fx = Fx::with_buffer(cx, 100);
    let view = fx.open(trickle(80, 70));
    fx.draw();
    search(&mut fx, "error");
    fx.click("log-search-filter");
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 16);

    tick(&mut fx, 5); // lines 80..85: line 84 is an error
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 17);
    assert_eq!(
        fx.read(&view, |v| v.row_text(16)).as_deref(),
        Some("ERROR line 84")
    );

    // 150 lines streamed: the ring keeps seqs 50..150 and the matches among the dropped ones
    // leave the front of the rows.
    tick(&mut fx, 65);
    let rows = fx.read(&view, |v| {
        (0..v.line_window().row_count())
            .filter_map(|i| v.row_text(i))
            .collect::<Vec<_>>()
    });
    assert!(rows[0].contains("older lines dropped"), "{rows:?}");
    assert!(rows[1..].iter().all(|r| r.starts_with("ERROR")), "{rows:?}");
    assert_eq!(rows[1], "ERROR line 54");
    assert_eq!(rows.last().map(String::as_str), Some("ERROR line 149"));
    // The count follows the index, which equals a scan of what is retained.
    let (matches, lines) = fx.read(&view, |v| {
        let c = v.search_counts();
        (c.matches, c.lines)
    });
    assert_eq!((matches, lines), (20, 100));
}

#[gpui::test]
fn next_skips_a_match_the_ring_dropped(cx: &mut TestAppContext) {
    let mut fx = Fx::with_buffer(cx, 100);
    let view = fx.open(trickle(80, 80));
    fx.draw();
    search(&mut fx, "error");
    fx.wheel(500.);
    fx.keys("enter"); // the first match
    assert_eq!(fx.read(&view, |v| v.search_state().current()), Some(4));
    tick(&mut fx, 80); // 160 lines streamed: seqs 60..160 retained, seq 4 is gone
    fx.keys("enter");
    let next = fx.read(&view, |v| v.search_state().current()).unwrap();
    assert_eq!(next, 64, "the next retained match, not the dropped one");
    assert!(fx.read(&view, |v| v.line_window().index().unwrap().contains(next)));
}

#[gpui::test]
fn a_large_buffer_is_searched_in_the_background_and_published_when_done(cx: &mut TestAppContext) {
    let mut fx = Fx::with_buffer(cx, 50_000);
    let view = fx.open(Timeline::immediate(lines(0, 12_000)).keep_open());
    fx.draw();
    fx.keys("/");
    // Typing `error` is five edits; set the text in one go to look at the state in between.
    fx.vcx.update(|window, cx| {
        view.update(cx, |v, cx| v.set_search_text("error", window, cx));
    });
    let scanning = fx.read(&view, |v| v.search_counts().scanning);
    assert!(scanning, "12 000 lines are not tested on the UI thread");
    assert_eq!(status(&mut fx, &view), "Searching…");
    fx.settle();
    let counts = fx.read(&view, |v| v.search_counts());
    assert!(!counts.scanning);
    assert_eq!((counts.matches, counts.lines), (2_400, 12_000));
    assert_eq!(status(&mut fx, &view), "2,400 matches");
}

#[gpui::test]
fn the_search_survives_closing_the_tab_and_reopening_the_pod_in_the_same_session(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 20);
    search(&mut fx, "error");
    fx.click("log-search-case");
    fx.click("log-search-filter");
    drop(view);

    // Close the tab: the view and its session go.
    let workspace = fx.workspace.clone();
    fx.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.close_active_item(window, cx));
    });
    fx.settle();
    assert_eq!(fx.vcx.update(|_, cx| workspace.read(cx).items().count()), 0);

    // The same pod again, in the same window: the filter is back, applied to the new stream.
    let view = open(&mut fx, 20);
    let state = fx.read(&view, |v| {
        let s = v.search_state();
        (
            s.is_open(),
            s.text().to_owned(),
            s.case_sensitive(),
            s.mode(),
        )
    });
    assert_eq!(state, (true, "error".to_owned(), true, SearchMode::Filter));
    assert!(fx.drawn("log-search"), "the bar is open");
    // `ERROR` is upper case and the search is case-sensitive: `error` finds nothing.
    assert_eq!(fx.read(&view, |v| v.search_counts().matches), 0);
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 0);

    // Closing the bar forgets it: a third open starts clean.
    fx.keys("escape");
    let workspace = fx.workspace.clone();
    fx.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.close_active_item(window, cx));
    });
    fx.settle();
    let view = open(&mut fx, 20);
    assert!(!fx.read(&view, |v| v.search_state().is_open()));
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 20);
}

#[gpui::test]
fn a_new_stream_starts_the_search_over_on_its_own_lines(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 20);
    search(&mut fx, "error");
    assert_eq!(fx.read(&view, |v| v.search_counts().matches), 4);
    // Reading the head instead reopens the stream: its seqs start at 0 again.
    fx.script(Timeline::immediate(lines(0, 10)).keep_open());
    fx.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.set_range(oxikube_domain::log::LogRange::Head, cx)
        });
    });
    fx.settle();
    let counts = fx.read(&view, |v| v.search_counts());
    assert_eq!((counts.matches, counts.lines), (2, 10));
}

#[gpui::test]
fn filtering_a_wrapped_view_keeps_its_list_in_step_with_the_stream(cx: &mut TestAppContext) {
    let mut fx = Fx::with_buffer(cx, 100);
    let view = fx.open(trickle(80, 70));
    fx.draw();
    fx.keys("w");
    search(&mut fx, "error");
    fx.click("log-search-filter");
    let consistent = |fx: &mut Fx| {
        fx.draw();
        fx.read(&view, |v| {
            assert_eq!(v.list.item_count(), v.line_window().row_count());
        });
    };
    consistent(&mut fx);
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 16);
    for _ in 0..7 {
        tick(&mut fx, 10);
        consistent(&mut fx);
    }
    // 150 lines streamed: the ring's 100 hold 20 matches (and the truncated marker is a row).
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 20);
    assert_eq!(fx.read(&view, |v| v.line_window().row_count()), 21);
    // Leaving filter mode makes every retained line a row again.
    fx.click("log-search-filter");
    consistent(&mut fx);
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 100);
}
