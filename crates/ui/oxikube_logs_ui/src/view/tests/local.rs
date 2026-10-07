//! Selecting, copying, marking and clearing (E08-S06): by pointer and by the shipped keys, over
//! the fake log port, with the fake clipboard.

use gpui::{Modifiers, MouseButton, Pixels, Point, TestAppContext};
use oxikube_domain::command::Command;
use oxikube_testkit::Timeline;

use super::fixture::{Fx, line, lines, pod_ref};
use super::scroll::{tick, trickle};

impl Fx {
    /// The window position of row `index` while the list is at the top (rows are 20 px).
    fn row_point(&mut self, index: usize) -> Point<Pixels> {
        let body = self
            .vcx
            .debug_bounds("log-body")
            .expect("the rows are drawn");
        body.origin + gpui::point(gpui::px(60.), gpui::px(20. * index as f32 + 10.))
    }

    /// Presses the left button on row `index` (shift held when `shift`) and lets go.
    fn click_row(&mut self, index: usize, shift: bool) {
        let at = self.row_point(index);
        let modifiers = Modifiers {
            shift,
            ..Modifiers::none()
        };
        self.vcx.simulate_click(at, modifiers);
        self.settle();
    }

    /// Drags from row `from` to row `to` with the left button down.
    fn drag_rows(&mut self, from: usize, to: usize) {
        let start = self.row_point(from);
        self.vcx
            .simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        for row in from..=to {
            let at = self.row_point(row);
            self.vcx
                .simulate_mouse_move(at, Some(MouseButton::Left), Modifiers::none());
        }
        let end = self.row_point(to);
        self.vcx
            .simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
        self.settle();
    }

    fn clipboard(&mut self) -> Option<String> {
        self.vcx.read_from_clipboard().and_then(|item| item.text())
    }

    pub(crate) fn toasts(&mut self) -> Vec<String> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .toast_layer()
                .read(cx)
                .visible()
                .iter()
                .map(|toast| toast.message.to_string())
                .collect()
        })
    }
}

fn open(fx: &mut Fx, n: usize) -> gpui::Entity<crate::LogView> {
    fx.open(Timeline::immediate(lines(0, n)).keep_open())
}

#[gpui::test]
fn click_selects_a_line_and_shift_click_extends_it(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 10);
    fx.click_row(2, false);
    assert_eq!(fx.read(&view, |v| v.selection()), Some(2..=2));
    fx.click_row(5, true);
    assert_eq!(fx.read(&view, |v| v.selection()), Some(2..=5));
    fx.click_row(0, true);
    assert_eq!(
        fx.read(&view, |v| v.selection()),
        Some(0..=2),
        "extended back past the anchor"
    );
    fx.click_row(7, false);
    assert_eq!(fx.read(&view, |v| v.selection()), Some(7..=7));
    fx.keys("escape");
    assert_eq!(fx.read(&view, |v| v.selection()), None);
}

#[gpui::test]
fn dragging_across_rows_selects_them(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 10);
    fx.drag_rows(1, 4);
    assert_eq!(fx.read(&view, |v| v.selection()), Some(1..=4));
}

#[gpui::test]
fn copy_puts_the_selected_lines_on_the_clipboard(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open(&mut fx, 10);
    fx.click_row(2, false);
    fx.click_row(4, true);
    fx.keys("c");
    assert_eq!(
        fx.clipboard().as_deref(),
        Some("INFO line 2\nINFO line 3\nERROR line 4\n"),
        "the raw text of the selected lines"
    );
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsCopy { target: pod_ref() }),
        "the key is the command"
    );
    assert!(
        fx.toasts().iter().any(|t| t == "Copied 3 lines"),
        "{:?}",
        fx.toasts()
    );

    // With the timestamps shown, the copy matches the screen.
    fx.keys("t");
    fx.keys("c");
    let text = fx.clipboard().unwrap();
    assert!(
        text.starts_with("2026-10-04T") && text.lines().next().unwrap().ends_with("INFO line 2"),
        "{text}"
    );
}

#[gpui::test]
fn with_no_selection_copy_takes_what_is_on_screen(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 5_000);
    fx.draw();
    // Follow the tail: the screen holds the newest lines.
    let seqs = fx
        .read(&view, |v| v.viewport_seqs())
        .expect("lines on screen");
    assert!(seqs.end == 5_000 && seqs.start > 4_000, "{seqs:?}");
    fx.keys("c");
    let text = fx.clipboard().unwrap();
    let copied: Vec<&str> = text.lines().collect();
    assert_eq!(copied.len() as u64, seqs.end - seqs.start);
    assert_eq!(copied.last().copied(), Some(line(4_999).text.as_str()));
    assert!(copied.len() < 200, "only the screen: {}", copied.len());

    // Scrolled up, the screen is other lines: the copy follows it.
    fx.wheel(300.);
    let moved = fx.read(&view, |v| v.viewport_seqs()).unwrap();
    assert!(moved.end < seqs.start);
    fx.keys("c");
    let text = fx.clipboard().unwrap();
    assert_eq!(text.lines().count() as u64, moved.end - moved.start);
}

#[gpui::test]
fn a_selection_stays_on_its_lines_when_the_list_scrolls_and_new_lines_arrive(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 400);
    fx.wheel(1000.); // to the top: autoscroll pauses
    fx.click_row(3, false);
    fx.click_row(5, true);
    assert_eq!(fx.read(&view, |v| v.selection()), Some(3..=5));
    fx.wheel(-40.);
    fx.wheel(20.);
    assert_eq!(
        fx.read(&view, |v| v.selection()),
        Some(3..=5),
        "scrolling does not reset the selection"
    );
    fx.keys("c");
    let expected: String = (3..=5).map(|i| format!("{}\n", line(i).text)).collect();
    assert_eq!(fx.clipboard(), Some(expected));
}

#[gpui::test]
fn a_selection_follows_its_lines_when_the_ring_drops_the_oldest(cx: &mut TestAppContext) {
    let mut fx = Fx::with_buffer(cx, 100);
    let view = fx.open(trickle(100, 40));
    fx.wheel(1_000.); // to the top: autoscroll pauses
    fx.click_row(2, false);
    fx.click_row(8, true);
    assert_eq!(fx.read(&view, |v| v.selection()), Some(2..=8));

    tick(&mut fx, 5); // seqs 0..5 leave the ring: every row index moves up by five
    assert_eq!(fx.read(&view, |v| v.line_window().first_seq()), 5);
    assert_eq!(
        fx.read(&view, |v| v.selection()),
        Some(2..=8),
        "the selection is by seq: it did not slide onto the lines that took the rows"
    );
    fx.keys("c");
    let expected: String = (5..=8).map(|i| format!("{}\n", line(i).text)).collect();
    assert_eq!(
        fx.clipboard(),
        Some(expected),
        "a copy reads what is still there"
    );

    // Entirely in what went: nothing is selected any more.
    fx.click_row(0, false); // seq 5
    tick(&mut fx, 10); // seqs 5..15 leave the ring
    assert!(fx.read(&view, |v| v.line_window().first_seq()) >= 15);
    assert_eq!(fx.read(&view, |v| v.selection()), None);
}

#[gpui::test]
fn m_marks_the_focused_line_and_the_bar_survives_scrolling(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 500);
    fx.wheel(1_000.); // to the top
    fx.click_row(4, false);
    fx.keys("m");
    assert_eq!(fx.read(&view, |v| v.marked()), [4]);
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsMark { target: pod_ref() })
    );
    assert!(fx.drawn("log-mark:4"), "the gutter bar is drawn");
    assert!(!fx.drawn("log-mark:5"));

    fx.wheel(-60.);
    assert!(!fx.drawn("log-mark:4"), "scrolled off screen");
    assert_eq!(fx.read(&view, |v| v.marked()), [4], "the mark stays");
    fx.wheel(1_000.);
    assert!(
        fx.drawn("log-mark:4"),
        "and is there when the line comes back"
    );

    // Another line, then both off again.
    fx.click_row(7, false);
    fx.keys("m");
    assert_eq!(fx.read(&view, |v| v.marked()), [4, 7]);
    fx.click_row(4, false);
    fx.keys("m");
    assert_eq!(fx.read(&view, |v| v.marked()), [7], "a second m unmarks");
    assert!(!fx.drawn("log-mark:4"));
}

#[gpui::test]
fn with_nothing_focused_m_marks_the_line_on_top(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 500);
    fx.wheel(1_000.);
    fx.keys("m");
    assert_eq!(fx.read(&view, |v| v.marked()), [0]);
    assert!(fx.drawn("log-mark:0"));
}

#[gpui::test]
fn a_mark_lives_only_while_its_line_is_in_the_buffer(cx: &mut TestAppContext) {
    let mut fx = Fx::with_buffer(cx, 100);
    let view = fx.open(trickle(100, 40));
    fx.wheel(1_000.);
    fx.click_row(2, false);
    fx.keys("m");
    fx.click_row(8, false);
    fx.keys("m");
    assert_eq!(fx.read(&view, |v| v.marked()), [2, 8]);
    tick(&mut fx, 5); // seqs 0..5 leave the ring
    let first = fx.read(&view, |v| v.line_window().first_seq());
    assert_eq!(first, 5);
    assert_eq!(
        fx.read(&view, |v| v.marked()),
        [8],
        "the mark on a dropped line went with it"
    );
}

#[gpui::test]
fn clear_empties_the_view_while_new_lines_keep_arriving(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(trickle(50, 6));
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 50);
    fx.keys("shift-c");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsClear { target: pod_ref() })
    );
    fx.read(&view, |v| {
        let window = v.line_window();
        assert_eq!(window.line_count(), 0, "the view is empty");
        assert!(
            !window.is_truncated(),
            "cleared lines are not dropped lines"
        );
        assert!(
            v.session().unwrap().is_empty(),
            "so is the session's buffer"
        );
    });
    tick(&mut fx, 3);
    fx.read(&view, |v| {
        let window = v.line_window();
        assert_eq!(window.line_count(), 3, "the stream went on");
        assert_eq!(window.first_seq(), 50, "from where it was");
        assert!(!window.is_truncated());
        assert_eq!(v.row_text(0).as_deref(), Some(line(50).text.as_str()));
    });
    // The cluster was not asked for anything again: one stream read the whole time.
    assert_eq!(fx.opened().len(), 1);
}

#[gpui::test]
fn clear_with_marks_asks_first(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(trickle(30, 0));
    fx.click_row(3, false);
    fx.keys("m");
    fx.keys("shift-c");
    assert!(fx.drawn("dialog-modal"), "a marked line makes it ask");
    assert_eq!(
        fx.read(&view, |v| v.line_window().line_count()),
        30,
        "nothing went yet"
    );
    fx.click("dialog-cancel");
    assert!(!fx.drawn("dialog-modal"));
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 30);
    assert_eq!(fx.read(&view, |v| v.marked()), [3]);

    fx.keys("shift-c");
    fx.click("dialog-confirm");
    fx.read(&view, |v| {
        assert_eq!(v.line_window().line_count(), 0);
        assert!(v.marked().is_empty(), "the marks go with the buffer");
        assert_eq!(v.selection(), None);
    });
}

#[gpui::test]
fn clear_without_marks_does_not_ask(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(trickle(30, 0));
    fx.click_row(3, false);
    fx.keys("shift-c");
    assert!(!fx.drawn("dialog-modal"));
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 0);
}
