//! The `terminal` font and cursor settings reach an open element: font changes re-lay it out once,
//! the cursor shape and blink are the default until the process asks for its own (E09-S11).

use gpui::TestAppContext;
use oxikube_terminal::grid::CursorShape;

use super::{Harness, configure, harness_themed};

fn cursor(h: &mut Harness) -> (CursorShape, bool) {
    let cursor = h
        .terminal
        .read_with(&mut *h.window, |terminal, _| terminal.snapshot().cursor);
    (cursor.shape, cursor.blinking)
}

#[gpui::test]
fn a_font_size_change_lays_the_terminal_out_again_once(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "font_size": 10 } }"#);
    let mut h = harness_themed(cx, 480., 130.);
    h.output("hello\r\nworld");
    assert_eq!(h.grid(), (80, 10), "6 x 13 px cells in 480 x 130 px");

    configure(cx, r#"{ "terminal": { "font_size": 20 } }"#);
    h.frame();
    assert_eq!(h.grid(), (40, 5), "12 x 26 px cells: half as many");
    let relaid = h.state.cache_stats();

    // Nothing else changed: further frames reuse every shaped row.
    h.frame();
    h.frame();
    let idle = h.state.cache_stats();
    assert_eq!(idle.misses, relaid.misses, "no further layout");
    assert_eq!(idle.shaped_runs, relaid.shaped_runs);
    assert_eq!(
        h.terminal
            .read_with(&mut *h.window, |t, _| t.snapshot().row_text(0)),
        "hello",
        "the content survived the reflow"
    );
}

#[gpui::test]
fn the_line_height_and_the_family_come_from_the_settings(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "font_size": 10 } }"#);
    let mut h = harness_themed(cx, 480., 130.);
    h.output("x");
    assert_eq!(h.grid().1, 10, "rows 13 px tall at line_height 1.3");

    configure(
        cx,
        r#"{ "terminal": { "font_size": 10, "line_height": 2.0 } }"#,
    );
    h.frame();
    assert_eq!(h.grid().1, 6, "rows 20 px tall");

    let before = h.state.cache_stats();
    configure(
        cx,
        r#"{ "terminal": { "font_size": 10, "line_height": 2.0, "font_family": "Courier" } }"#,
    );
    h.frame();
    let after = h.state.cache_stats();
    assert!(
        after.misses > before.misses,
        "a new family shapes the rows again"
    );
    h.frame();
    assert_eq!(h.state.cache_stats().misses, after.misses, "once");
}

#[gpui::test]
fn without_a_font_setting_the_theme_decides(cx: &mut TestAppContext) {
    configure(cx, "{}");
    let mut h = harness_themed(cx, 480., 130.);
    h.output("x");
    let themed = h.grid();
    configure(cx, r#"{ "terminal": { "font_size": 14 } }"#);
    h.frame();
    assert_ne!(h.grid(), themed, "an explicit size replaces the theme's");
    // Out-of-range sizes are clamped (6 - 72 points), never refused.
    configure(cx, r#"{ "terminal": { "font_size": 1 } }"#);
    h.frame();
    let smallest = h.grid();
    configure(cx, r#"{ "terminal": { "font_size": 6 } }"#);
    h.frame();
    assert_eq!(h.grid(), smallest, "1 behaves as 6");
}

#[gpui::test]
fn the_cursor_setting_is_the_default_and_the_process_may_override_it(cx: &mut TestAppContext) {
    configure(
        cx,
        r#"{ "terminal": { "cursor_shape": "bar", "cursor_blink": true } }"#,
    );
    let mut h = harness_themed(cx, 480., 130.);
    h.output("x");
    assert_eq!(cursor(&mut h), (CursorShape::Beam, true));

    // Hot reload: a steady underline.
    configure(
        cx,
        r#"{ "terminal": { "cursor_shape": "underline", "cursor_blink": false } }"#,
    );
    h.frame();
    assert_eq!(cursor(&mut h), (CursorShape::Underline, false));

    // The process asks for a steady block (DECSCUSR 2) and later resets it.
    h.output("\x1b[2 q");
    assert_eq!(cursor(&mut h), (CursorShape::Block, false));
    h.output("\x1b[0 q");
    assert_eq!(cursor(&mut h), (CursorShape::Underline, false));
}

#[gpui::test]
fn a_blinking_cursor_flips_on_the_clock_and_typing_keeps_it_on(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "cursor_blink": true } }"#);
    let mut h = harness_themed(cx, 480., 130.);
    h.output("x");
    assert!(h.state.cursor_on());
    assert!(h.state.blink_tick(), "it blinks on screen: repaint");
    assert!(!h.state.cursor_on(), "off phase");
    // Typing shows it at once.
    h.window.simulate_input("a");
    assert!(h.state.cursor_on());

    // Not painted since the last tick (a hidden tab): no repaint is asked for.
    h.frame();
    assert!(h.state.blink_tick());
    assert!(!h.state.blink_tick(), "nothing painted in between");
    assert!(h.state.cursor_on(), "an unpainted terminal rests on");
}

#[gpui::test]
fn a_steady_cursor_never_asks_for_repaints(cx: &mut TestAppContext) {
    configure(cx, r#"{ "terminal": { "cursor_blink": false } }"#);
    let mut h = harness_themed(cx, 480., 130.);
    h.output("x");
    assert!(!h.state.blink_tick());
    assert!(h.state.cursor_on());
}
