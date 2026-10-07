//! JSON mode (E08-S05): columns for structured lines, plain lines untouched in the same stream,
//! expanding a line into pretty JSON, and the level chips.

use std::time::Duration;

use gpui::{Entity, TestAppContext};
use oxikube_domain::command::Command;
use oxikube_domain::log::{LevelChip, LogLevel, LogLine};
use oxikube_testkit::Timeline;

use super::fixture::{Fx, pod_ref, ts};
use crate::LogView;

/// A zap-style line.
fn zap(i: usize, level: &str, msg: &str) -> LogLine {
    LogLine::new(
        ts(i),
        "web-0",
        "app",
        format!(
            r#"{{"level":"{level}","ts":1791115200.{i:03},"caller":"main.go:{i}","msg":"{msg}","n":{i}}}"#
        ),
    )
}

fn plain(i: usize, text: &str) -> LogLine {
    LogLine::new(ts(i), "web-0", "app", text)
}

/// banner, info, debug, plain, warn, error, a half-written JSON line, debug.
fn mixed() -> Vec<LogLine> {
    vec![
        plain(0, "=== booting service ==="),
        zap(1, "info", "listening"),
        zap(2, "debug", "cache warmed"),
        plain(3, "ERROR plain text that mentions a level"),
        zap(4, "warn", "slow request"),
        zap(5, "error", "boom"),
        plain(6, r#"{"level":"info","msg":"cut o"#),
        zap(7, "debug", "tick"),
    ]
}

fn open_mixed(fx: &mut Fx) -> Entity<LogView> {
    fx.open(Timeline::immediate(mixed()).keep_open())
}

fn rows(fx: &mut Fx, view: &Entity<LogView>) -> Vec<String> {
    fx.read(view, |v| {
        (0..v.line_window().row_count())
            .map(|i| v.row_text(i).unwrap_or_default())
            .collect()
    })
}

#[gpui::test]
fn structured_lines_render_as_columns_and_plain_lines_stay_text(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_mixed(&mut fx);
    fx.read(&view, |v| {
        // Plain text, even one that names a level or looks like cut-off JSON, is just text.
        assert_eq!(v.row_text(0).as_deref(), Some("=== booting service ==="));
        assert!(v.row_columns(0).is_none());
        assert!(v.row_columns(3).is_none());
        assert_eq!(
            v.row_text(6).as_deref(),
            Some(r#"{"level":"info","msg":"cut o"#)
        );
        assert!(v.row_columns(6).is_none());
        // A structured line: level, time, message and the other fields collapsed.
        let columns = v.row_columns(1).expect("a JSON row");
        assert_eq!(columns.level, LogLevel::Info);
        assert_eq!(columns.time, "12:00:00.001");
        assert_eq!(columns.message, "listening");
        assert_eq!(columns.summary, "caller=main.go:1 n=1");
        assert_eq!(
            v.row_text(1).as_deref(),
            Some("INFO 12:00:00.001 listening caller=main.go:1 n=1")
        );
        assert_eq!(v.row_columns(5).unwrap().level, LogLevel::Error);
        // Copy takes the line as written, not the columns.
        assert_eq!(
            v.raw_text(1).as_deref(),
            Some(r#"{"level":"info","ts":1791115200.001,"caller":"main.go:1","msg":"listening","n":1}"#)
        );
    });
    assert!(fx.drawn("log-row:1"), "the JSON row is drawn");
    assert!(fx.drawn("log-levels"), "the chips are shown in JSON mode");
}

#[gpui::test]
fn clicking_a_json_row_expands_it_into_pretty_json_and_again_collapses_it(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_mixed(&mut fx);
    assert!(!fx.drawn("log-detail"));
    fx.click("log-row:1");
    fx.read(&view, |v| {
        assert_eq!(v.expanded_seq(), Some(1));
        assert_eq!(
            v.expanded_text().as_deref(),
            Some(
                "{\n  \"level\": \"info\",\n  \"ts\": 1791115200.001,\n  \"caller\": \"main.go:1\",\n  \"msg\": \"listening\",\n  \"n\": 1\n}"
            )
        );
    });
    assert!(fx.drawn("log-detail"));
    // Another line moves the pane; the same one closes it.
    fx.click("log-row:5");
    assert_eq!(fx.read(&view, |v| v.expanded_seq()), Some(5));
    fx.click("log-row:5");
    assert_eq!(fx.read(&view, |v| v.expanded_seq()), None);
    assert!(!fx.drawn("log-detail"));
    // The close button does too.
    fx.click("log-row:2");
    assert!(fx.drawn("log-detail"));
    fx.vcx
        .update(|_, cx| view.update(cx, |v, cx| v.collapse(cx)));
    fx.settle();
    assert!(!fx.drawn("log-detail"));
}

#[gpui::test]
fn expanding_and_closing_a_line_go_through_commands(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_mixed(&mut fx);
    fx.click("log-row:1");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsToggleLine {
            target: pod_ref(),
            seq: 1
        })
    );
    assert_eq!(fx.read(&view, |v| v.expanded_seq()), Some(1));
    fx.click("log-detail-close");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsCollapseLine { target: pod_ref() })
    );
    assert_eq!(fx.read(&view, |v| v.expanded_seq()), None);
    assert!(!fx.drawn("log-detail"));
}

#[gpui::test]
fn the_pane_closes_when_json_mode_goes_off_or_a_chip_hides_its_line(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_mixed(&mut fx);
    fx.click("log-row:1");
    assert!(fx.drawn("log-detail"));
    fx.keys("j");
    assert!(fx.read(&view, |v| !v.options().json));
    assert_eq!(fx.read(&view, |v| v.expanded_seq()), None);
    assert!(!fx.drawn("log-detail"), "raw text mode has no pane");
    fx.keys("j");
    assert_eq!(fx.read(&view, |v| v.expanded_seq()), None);

    // A chip that keeps the line leaves the pane; one that hides it closes it.
    fx.click("log-row:1");
    fx.click("log-level-debug");
    assert_eq!(fx.read(&view, |v| v.expanded_seq()), Some(1));
    fx.click("log-level-info");
    assert_eq!(fx.read(&view, |v| v.expanded_seq()), None);
    assert!(!fx.drawn("log-detail"));
}

#[gpui::test]
fn plain_lines_do_not_expand(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_mixed(&mut fx);
    fx.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.toggle_expanded(0, cx);
            v.toggle_expanded(6, cx);
            v.toggle_expanded(999, cx);
        })
    });
    assert_eq!(fx.read(&view, |v| v.expanded_seq()), None);
}

#[gpui::test]
fn a_level_chip_hides_its_lines_and_plain_lines_stay_visible(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_mixed(&mut fx);
    assert_eq!(rows(&mut fx, &view).len(), 8);

    fx.click("log-level-debug");
    // The chip went through its command.
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsToggleLevel {
            target: pod_ref(),
            level: LevelChip::Debug
        })
    );
    let shown = rows(&mut fx, &view);
    assert_eq!(shown.len(), 6, "two debug lines are hidden: {shown:?}");
    assert!(shown.iter().all(|row| !row.contains("DEBUG")));
    // Every plain line (banner, the one that says ERROR, the cut-off JSON) is still there.
    assert_eq!(shown[0], "=== booting service ===");
    assert!(shown.iter().any(|r| r.starts_with("ERROR plain text")));
    assert!(
        shown
            .iter()
            .any(|r| r.starts_with(r#"{"level":"info","msg":"cut o"#))
    );
    assert!(fx.read(&view, |v| !v.levels().shows(LevelChip::Debug)));

    // `text` hides the plain lines and nothing else.
    fx.click("log-level-text");
    let shown = rows(&mut fx, &view);
    assert_eq!(shown.len(), 3, "info, warn, error: {shown:?}");
    assert!(shown[0].starts_with("INFO") && shown[2].starts_with("ERROR"));

    // Turning them back on restores the stream, in order.
    fx.click("log-level-debug");
    fx.click("log-level-text");
    let all = rows(&mut fx, &view);
    assert_eq!(all.len(), 8);
    assert_eq!(all[0], "=== booting service ===");
    assert!(all[7].starts_with("DEBUG"));
    assert!(fx.read(&view, |v| v.levels().is_all()));
}

#[gpui::test]
fn lines_that_arrive_after_a_chip_is_off_are_filtered_too(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let clock = fx.ports.logs.clock().clone();
    let step = Duration::from_secs(1);
    let view = fx.open(
        Timeline::new()
            .ok_at(Duration::ZERO, zap(0, "info", "first"))
            .ok_at(step, zap(1, "debug", "noise"))
            .ok_at(step * 2, plain(2, "plain later"))
            .ok_at(step * 3, zap(3, "error", "later error"))
            .keep_open(),
    );
    fx.click("log-level-debug");
    for _ in 0..3 {
        clock.advance(step);
        fx.settle();
    }
    let shown = rows(&mut fx, &view);
    assert_eq!(shown.len(), 3, "{shown:?}");
    assert!(shown[0].contains("first"));
    assert_eq!(shown[1], "plain later");
    assert!(shown[2].contains("later error"));
    // The window holds all four lines, shows three.
    fx.read(&view, |v| {
        assert_eq!(v.line_window().retained_count(), 4);
        assert_eq!(v.line_window().line_count(), 3);
    });
}

#[gpui::test]
fn json_mode_off_shows_raw_text_without_filtering(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_mixed(&mut fx);
    fx.click("log-level-error");
    assert_eq!(rows(&mut fx, &view).len(), 7);

    // `j` (the shipped keymap) goes through logs::ToggleJsonMode.
    fx.keys("j");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsToggleJsonMode { target: pod_ref() })
    );
    assert!(fx.read(&view, |v| !v.options().json));
    let raw = rows(&mut fx, &view);
    assert_eq!(raw.len(), 8, "nothing is hidden in raw mode");
    assert_eq!(
        raw[1],
        r#"{"level":"info","ts":1791115200.001,"caller":"main.go:1","msg":"listening","n":1}"#
    );
    assert!(fx.read(&view, |v| v.row_columns(1).is_none()));
    assert!(!fx.drawn("log-levels"), "no chips in raw mode");

    // Back on: the columns and the chips (with the error chip still off) return.
    fx.keys("j");
    assert!(fx.read(&view, |v| v.options().json));
    assert_eq!(rows(&mut fx, &view).len(), 7);
    assert!(fx.read(&view, |v| v.row_columns(1).is_some()));
    assert!(fx.drawn("log-levels"));
}

#[gpui::test]
fn the_filter_survives_a_reopened_stream(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_mixed(&mut fx);
    fx.click("log-level-debug");
    fx.script(Timeline::immediate(mixed()).keep_open());
    fx.keys("2");
    assert_eq!(fx.opened().len(), 2);
    let shown = rows(&mut fx, &view);
    assert_eq!(
        shown.len(),
        6,
        "the new session is filtered from its first line: {shown:?}"
    );
    assert!(fx.read(&view, |v| v.expanded_seq().is_none()));
}

#[gpui::test]
fn the_truncated_marker_and_the_filter_work_together(cx: &mut TestAppContext) {
    let mut fx = Fx::with_buffer(cx, 100);
    let lines: Vec<LogLine> = (0..250)
        .map(|i| zap(i, if i % 2 == 0 { "info" } else { "debug" }, "m"))
        .collect();
    // 250 lines into a 100-line buffer: seqs 150..250 are kept.
    let view = fx.open(Timeline::immediate(lines).keep_open());
    fx.click("log-level-debug");
    fx.read(&view, |v| {
        let window = v.line_window();
        assert_eq!(window.retained_count(), 100);
        assert!(window.is_truncated());
        // Only the info lines (even seqs) of the retained ones are rows, under the marker.
        assert_eq!(window.line_count(), 50);
        assert_eq!(window.row_count(), 51);
        assert!(v.row_text(0).unwrap().contains("older lines dropped"));
        assert!(v.row_text(1).unwrap().contains("INFO"));
    });
}

#[gpui::test]
fn the_expanded_line_closes_when_the_buffer_drops_it(cx: &mut TestAppContext) {
    let mut fx = Fx::with_buffer(cx, 100);
    let clock = fx.ports.logs.clock().clone();
    let step = Duration::from_secs(1);
    let mut timeline = Timeline::new().ok_at(Duration::ZERO, zap(0, "info", "oldest"));
    for i in 1..=120 {
        timeline = timeline.ok_at(step, zap(i, "info", "m"));
    }
    let view = fx.open(timeline.keep_open());
    fx.click("log-row:0");
    assert_eq!(fx.read(&view, |v| v.expanded_seq()), Some(0));
    clock.advance(step);
    fx.settle();
    assert_eq!(
        fx.read(&view, |v| v.expanded_seq()),
        None,
        "line 0 left the ring buffer"
    );
}

#[gpui::test]
fn only_the_visible_json_rows_are_parsed(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let lines: Vec<LogLine> = (0..5_000).map(|i| zap(i, "info", "bulk")).collect();
    let view = fx.open(Timeline::immediate(lines).keep_open());
    fx.draw();
    let (built, parsed) = fx.read(&view, |v| (v.rows_built(), v.parsed_rows()));
    assert!(built > 0 && built < 200, "{built} rows built");
    assert!(
        parsed <= built.max(1) * 2,
        "{parsed} lines parsed for {built} rows built: the rest of the buffer is not parsed"
    );
    // A second frame over the same rows parses nothing new.
    fx.draw();
    assert_eq!(fx.read(&view, |v| v.parsed_rows()), parsed);
}

#[gpui::test]
fn the_wrapped_list_follows_the_filter_and_the_stream(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let clock = fx.ports.logs.clock().clone();
    let step = Duration::from_secs(1);
    let view = fx.open(
        Timeline::new()
            .ok_at(Duration::ZERO, zap(0, "info", "a"))
            .ok_at(Duration::ZERO, zap(1, "debug", "b"))
            .ok_at(Duration::ZERO, plain(2, "c"))
            .ok_at(step, zap(3, "debug", "d"))
            .ok_at(step, zap(4, "error", "e"))
            .keep_open(),
    );
    fx.keys("w");
    assert!(fx.read(&view, |v| v.options().wrap));
    fx.click("log-level-debug");
    let in_step = |fx: &mut Fx| {
        fx.read(&view, |v| {
            assert_eq!(
                v.list.item_count(),
                v.line_window().row_count(),
                "the wrapped list has one item per row"
            );
        })
    };
    in_step(&mut fx);
    assert_eq!(rows(&mut fx, &view).len(), 2);
    clock.advance(step);
    fx.settle();
    in_step(&mut fx);
    let shown = rows(&mut fx, &view);
    assert_eq!(shown.len(), 3, "{shown:?}");
    assert!(shown[2].contains('e'));
    // Chips back on, then JSON mode off: the list is rebuilt each time.
    fx.click("log-level-debug");
    in_step(&mut fx);
    assert_eq!(rows(&mut fx, &view).len(), 5);
    fx.keys("j");
    in_step(&mut fx);
}

#[gpui::test]
fn the_json_controls_appear_with_the_first_structured_line(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let clock = fx.ports.logs.clock().clone();
    let step = Duration::from_secs(1);
    let view = fx.open(
        Timeline::new()
            .ok_at(Duration::ZERO, plain(0, "plain one"))
            .ok_at(
                Duration::ZERO,
                plain(1, "level=info msg=logfmt is not JSON"),
            )
            .ok_at(step, zap(2, "info", "now structured"))
            .keep_open(),
    );
    assert!(!fx.drawn("log-json"), "a plain-text log has no JSON toggle");
    assert!(!fx.drawn("log-levels"), "and no chips");
    clock.advance(step);
    fx.settle();
    assert!(fx.read(&view, |v| v.line_window().line_count() == 3));
    assert!(fx.drawn("log-json"));
    assert!(fx.drawn("log-levels"));
}

#[gpui::test]
fn the_level_chips_compose_with_the_searchs_filter_mode(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_mixed(&mut fx);
    fx.draw();
    let line_count = |fx: &mut Fx| fx.read(&view, |v| v.line_window().line_count());
    fx.keys("/");
    fx.vcx.simulate_keystrokes("c a c h e");
    fx.settle();
    fx.click("log-search-filter");
    assert_eq!(line_count(&mut fx), 1, "only `cache warmed` matches");
    // Hiding debug hides that match: filter and chips are both in force.
    fx.click("log-level-debug");
    assert_eq!(line_count(&mut fx), 0);
    // Showing it again brings the match back; closing the search restores the other lines.
    fx.click("log-level-debug");
    assert_eq!(line_count(&mut fx), 1);
    fx.click("log-search-filter");
    assert_eq!(line_count(&mut fx), 8);
    fx.click("log-level-debug");
    assert_eq!(line_count(&mut fx), 6, "the two debug lines are hidden");
}

/// A structured line is selected and marked like any other (E08-S06): its row carries the
/// gutter bar, and a copy takes its raw JSON text, not the columns.
#[gpui::test]
fn a_structured_line_can_be_selected_marked_and_copied(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open_mixed(&mut fx);
    fx.draw();
    fx.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.click_line(1, false, cx);
            v.toggle_mark(cx);
        })
    });
    fx.draw();
    assert_eq!(fx.read(&view, |v| v.marked()), [1]);
    assert!(fx.drawn("log-mark:1"), "the gutter bar is on the JSON row");
    fx.vcx
        .update(|_, cx| view.update(cx, |v, cx| v.copy_lines(cx)));
    let copied = fx.vcx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(
        copied.as_deref(),
        Some(format!("{}\n", mixed()[1].text).as_str())
    );
}
