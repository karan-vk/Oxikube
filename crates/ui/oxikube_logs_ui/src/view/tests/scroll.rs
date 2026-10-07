//! Autoscroll, the "N new lines" pill, and the anchor line across the wrap toggle.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_testkit::Timeline;

use super::fixture::{Fx, line};

/// A stream that delivers `first` lines at once, then one more line every second, forever.
fn trickle(first: usize, more: usize) -> Timeline<oxikube_domain::log::LogLine> {
    let mut timeline = Timeline::new();
    for i in 0..first {
        timeline = timeline.ok_at(Duration::ZERO, line(i));
    }
    for i in 0..more {
        timeline = timeline.ok_at(Duration::from_secs(i as u64 + 1), line(first + i));
    }
    timeline.keep_open()
}

fn tick(fx: &mut Fx, seconds: u64) {
    for _ in 0..seconds {
        fx.ports.logs.clock().advance(Duration::from_secs(1));
        fx.settle();
    }
}

#[gpui::test]
fn autoscroll_follows_new_lines(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(trickle(200, 5));
    fx.draw();
    let (top, at_end) = fx.read(&view, |v| (v.top_row(), v.at_end()));
    assert!(
        top > 100 && at_end,
        "scrolled to the newest of 200 rows, top row {top}"
    );
    tick(&mut fx, 5);
    fx.draw();
    let (later, at_end, newest_seq) = fx.read(&view, |v| {
        (v.top_row(), v.at_end(), v.line_window().next_seq())
    });
    assert_eq!(newest_seq, 205);
    assert!(
        at_end && later == top + 5,
        "still at the end: top {later}, was {top}"
    );
    assert!(fx.read(&view, |v| v.autoscroll()));
    assert!(!fx.drawn("log-new-lines:0"));
}

#[gpui::test]
fn scrolling_up_pauses_and_the_pill_counts_new_lines_until_clicked(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(trickle(200, 7));
    fx.draw();
    fx.wheel(10.);
    assert!(!fx.read(&view, |v| v.autoscroll()), "a scroll up pauses");
    let anchor = fx.read(&view, |v| v.top_seq());

    tick(&mut fx, 3);
    fx.draw();
    assert_eq!(fx.read(&view, |v| v.new_lines()), 3);
    assert!(fx.drawn("log-new-lines:3"), "the pill says 3 new lines");
    assert_eq!(
        fx.read(&view, |v| v.top_seq()),
        anchor,
        "the lines on screen stay while paused"
    );

    fx.click("log-new-lines:3");
    assert!(fx.read(&view, |v| v.autoscroll()), "the pill resumes");
    assert_eq!(fx.read(&view, |v| v.new_lines()), 0);
    assert!(
        fx.dispatcher
            .sent()
            .iter()
            .any(|c| c.id() == oxikube_domain::command::CommandId::LOGS_TOGGLE_AUTOSCROLL)
    );
    tick(&mut fx, 2);
    fx.draw();
    let (rows, at_end) = fx.read(&view, |v| (v.line_window().row_count(), v.at_end()));
    assert!(at_end, "back at the end of {rows} rows");
}

#[gpui::test]
fn the_pill_count_survives_the_ring_buffer_dropping_lines(cx: &mut TestAppContext) {
    let mut fx = Fx::with_buffer(cx, 100);
    let view = fx.open(trickle(100, 30));
    fx.draw();
    fx.wheel(5.);
    tick(&mut fx, 30);
    fx.read(&view, |v| {
        assert_eq!(
            v.line_window().line_count(),
            100,
            "the buffer stays bounded"
        );
        assert_eq!(v.new_lines(), 30, "counted by seq, not by rows");
    });
}

#[gpui::test]
fn the_wrap_toggle_keeps_the_anchor_line(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(trickle(300, 0));
    fx.draw();
    fx.wheel(40.);
    fx.draw();
    let anchor = fx.read(&view, |v| v.top_seq()).expect("a line on top");
    assert!(anchor < 290, "scrolled up to {anchor}");

    fx.keys("w");
    fx.draw();
    fx.read(&view, |v| {
        assert!(v.options().wrap);
        assert_eq!(v.top_seq(), Some(anchor), "same line on top, wrapped");
    });

    fx.keys("w");
    fx.draw();
    fx.read(&view, |v| {
        assert!(!v.options().wrap);
        assert_eq!(v.top_seq(), Some(anchor), "same line on top, unwrapped");
    });
}

#[gpui::test]
fn wrapped_rows_follow_the_stream_too(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(trickle(50, 3));
    fx.keys("w");
    tick(&mut fx, 3);
    fx.draw();
    fx.read(&view, |v| {
        assert!(v.options().wrap && v.autoscroll());
        assert_eq!(v.line_window().row_count(), 53);
        assert!(
            v.rows_built() > 0 && v.rows_built() < 53,
            "{}",
            v.rows_built()
        );
        assert!(v.top_row() > 0 && v.at_end(), "at the end");
    });
}
