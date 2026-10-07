//! Rows that did not change are not laid out or shaped again.

use gpui::{TestAppContext, px};

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

#[gpui::test]
fn the_block_cursor_glyph_is_reshaped_on_zoom_and_palette_changes(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    // The cursor back over the `c`: a focused block with a glyph under it.
    h.output("abc\x1b[D");
    let first = h.state.cache_stats().cursor_glyphs;
    assert!(first >= 1, "the glyph under the block is shaped");
    h.window.draw_frame();
    h.window.draw_frame();
    assert_eq!(
        h.state.cache_stats().cursor_glyphs,
        first,
        "idle frames reuse it"
    );

    // Zoom: the cell under the cursor is the same, the font size is not.
    h.window.update_root(|host, _, cx| {
        host.font.size = px(FONT_SIZE * 2.);
        cx.notify();
    });
    h.frame();
    let cursor = h
        .terminal
        .read_with(&mut *h.window, |t, _| t.snapshot().cursor);
    assert_eq!(
        (cursor.row, cursor.column),
        (0, 2),
        "the cursor did not move"
    );
    let zoomed = h.state.cache_stats().cursor_glyphs;
    assert_eq!(zoomed, first + 1, "reshaped at the new size");

    // OSC 11 redefines the background the glyph is drawn in.
    h.output("\x1b]11;rgb:ff/00/00\x1b\\");
    assert_eq!(
        h.state.cache_stats().cursor_glyphs,
        zoomed + 1,
        "reshaped in the new colour"
    );
}
