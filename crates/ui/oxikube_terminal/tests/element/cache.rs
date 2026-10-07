//! Rows that did not change are not laid out or shaped again.

use gpui::TestAppContext;

use super::{FONT_SIZE, harness};

#[gpui::test]
fn unchanged_rows_are_not_reshaped(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("first line\r\n\x1b[31msecond\x1b[0m line\r\nthird");
    let before = h.state.cache_stats();
    assert!(before.shaped_runs >= 4, "{before:?}");

    // A repaint with nothing new: every row comes from the cache.
    h.window.draw_frame();
    h.window.draw_frame();
    let idle = h.state.cache_stats();
    assert_eq!(idle.shaped_runs, before.shaped_runs, "nothing reshaped");
    assert_eq!(idle.misses, before.misses);
    assert_eq!(idle.hits, before.hits + 2 * 10);

    // New output on the third row: that row (and the cursor's) is rebuilt, the others are not.
    h.output(" and more");
    let after = h.state.cache_stats();
    assert_eq!(after.misses, idle.misses + 1, "{after:?}");
    assert_eq!(after.shaped_runs, idle.shaped_runs + 1);
    assert_eq!(
        h.terminal
            .read_with(&mut *h.window, |t, _| t.snapshot().row_text(2)),
        "third and more"
    );
}

#[gpui::test]
fn scrolled_lines_keep_their_shaped_runs(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    let lines: String = (0..20).map(|i| format!("line {i}\r\n")).collect();
    h.output(&lines);
    let before = h.state.cache_stats();
    // Every row moves up by one; only the new line is shaped.
    h.output("line 20\r\n");
    let after = h.state.cache_stats();
    assert_eq!(after.misses, before.misses + 1, "{after:?}");
    assert_eq!(after.shaped_runs, before.shaped_runs + 1);
}
