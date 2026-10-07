//! The `LogView` key context with the shipped keymap: k9s's keys dispatch the `logs::*` commands,
//! which change the view's options and reopen its session.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::log::LogRange;
use oxikube_ports::LogSince;
use oxikube_testkit::Timeline;

use super::fixture::{Fx, lines, pod_ref};
use crate::view::{Copy, HEAD_LIMIT_BYTES, Mark};

fn open(fx: &mut Fx) -> gpui::Entity<crate::LogView> {
    let view = fx.open(Timeline::immediate(lines(0, 5)).keep_open());
    // Every key below that reopens the stream reads a fresh timeline.
    for _ in 0..12 {
        fx.script(Timeline::immediate(lines(0, 5)).keep_open());
    }
    view
}

#[gpui::test]
fn digits_set_the_range_and_reopen_the_stream(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    let cases = [
        ("1", LogRange::Head),
        ("2", LogRange::Last1m),
        ("3", LogRange::Last5m),
        ("4", LogRange::Last15m),
        ("5", LogRange::Last30m),
        ("6", LogRange::Last1h),
        ("0", LogRange::Tail),
    ];
    for (key, range) in cases {
        fx.keys(key);
        assert_eq!(fx.read(&view, |v| v.options().range), range, "key {key}");
        let sent = fx.dispatcher.sent();
        assert_eq!(
            sent.last(),
            Some(&Command::LogsSetRange {
                target: pod_ref(),
                range
            }),
            "key {key} dispatches logs::SetRange"
        );
        let opened = fx.opened();
        let last = opened.last().unwrap();
        match range {
            LogRange::Head => {
                assert!(!last.follow);
                assert_eq!(last.limit_bytes, Some(HEAD_LIMIT_BYTES));
            }
            LogRange::Tail => assert_eq!(last.tail_lines, Some(crate::view::TAIL_LINES)),
            since => assert_eq!(
                last.since,
                since.since_seconds().map(LogSince::Seconds),
                "key {key}"
            ),
        }
    }
    assert_eq!(fx.opened().len(), 8, "the first open and one per change");
    // The new session reads its own lines; the view shows them.
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 5);
}

#[gpui::test]
fn s_w_t_p_toggle_their_option_through_their_command(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    let target = pod_ref();

    fx.keys("t");
    assert!(fx.read(&view, |v| v.options().timestamps));
    let text = fx.read(&view, |v| v.row_text(0)).unwrap();
    assert!(
        text.starts_with("2026-10-04T") && text.ends_with("INFO line 0"),
        "the timestamp leads the row: {text}"
    );
    fx.keys("w");
    assert!(fx.read(&view, |v| v.options().wrap));
    fx.keys("s");
    assert!(!fx.read(&view, |v| v.autoscroll()));
    fx.keys("s");
    assert!(fx.read(&view, |v| v.autoscroll()));
    let opened_before = fx.opened().len();
    fx.keys("p");
    assert!(fx.read(&view, |v| v.options().previous));
    let last = fx.opened().last().cloned().unwrap();
    assert!(
        last.previous && !last.follow,
        "the previous instance is read"
    );
    assert_eq!(fx.opened().len(), opened_before + 1);

    let sent = fx.dispatcher.sent();
    assert_eq!(
        sent,
        [
            Command::LogsToggleTimestamps {
                target: target.clone()
            },
            Command::LogsToggleWrap {
                target: target.clone()
            },
            Command::LogsToggleAutoscroll {
                target: target.clone()
            },
            Command::LogsToggleAutoscroll {
                target: target.clone()
            },
            Command::LogsTogglePrevious { target },
        ]
    );
    // The display toggles reopen nothing.
    assert_eq!(opened_before, 1);
}

#[gpui::test]
fn f_fills_the_tab_and_m_c_are_reserved(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    fx.keys("f");
    assert!(fx.read_workspace_zoomed(), "the view's pane is zoomed");
    assert!(fx.vcx.update(|_, cx| view.read(cx).is_fullscreen(cx)));
    fx.keys("f");
    assert!(!fx.read_workspace_zoomed());

    // `m` and `c` are bound in the `LogView` context and the view handles them (reserved for
    // E08-S06's marks and copy, which dispatch nothing yet).
    fx.draw();
    let reserved: [(&dyn gpui::Action, &str); 2] = [(&Mark, "m"), (&Copy, "c")];
    for (action, key) in reserved {
        let (keys, available) = fx.vcx.update(|window, cx| {
            let focus = view.read(cx).focus.clone();
            let keys: Vec<String> = window
                .bindings_for_action_in(action, &focus)
                .iter()
                .map(|binding| {
                    let strokes: Vec<String> =
                        binding.keystrokes().iter().map(|k| k.unparse()).collect();
                    strokes.join(" ")
                })
                .collect();
            (keys, window.is_action_available_in(action, &focus))
        });
        assert!(keys.iter().any(|k| k == key), "{key} is bound: {keys:?}");
        assert!(available, "the view handles {}", action.name());
    }
    let sent = fx.dispatcher.sent().len();
    fx.keys("m c");
    assert_eq!(
        fx.dispatcher.sent().len(),
        sent,
        "mark and copy arrive with E08-S06"
    );
}

#[gpui::test]
fn the_toolbar_sends_the_same_commands(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    fx.click("log-range-15m");
    assert_eq!(fx.read(&view, |v| v.options().range), LogRange::Last15m);
    fx.click("log-wrap");
    assert!(fx.read(&view, |v| v.options().wrap));
    fx.click("log-timestamps");
    assert!(fx.read(&view, |v| v.options().timestamps));
    assert_eq!(
        fx.dispatcher.sent()[0],
        Command::LogsSetRange {
            target: pod_ref(),
            range: LogRange::Last15m
        }
    );
}

impl Fx {
    fn read_workspace_zoomed(&mut self) -> bool {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| workspace.read(cx).is_zoomed(cx))
    }
}
