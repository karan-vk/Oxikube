//! Scrollback limit, scrolling and resize reflow.

use oxikube_ports::TerminalSize;

use super::super::{MAX_SCROLLBACK_LINES, TermGrid, TerminalScroll};
use super::{feed, grid, screen};

fn lines(grid: &mut TermGrid, count: usize) {
    for line in 0..count {
        feed(grid, format!("line {line}\r\n"));
    }
}

#[test]
fn scrollback_limit_is_honoured() {
    let mut grid = grid(20, 5, 10);
    lines(&mut grid, 100);
    assert_eq!(grid.history_size(), 10);
    assert_eq!(grid.scrollback_limit(), 10);
    grid.scroll(TerminalScroll::Top);
    let snap = grid.snapshot();
    assert_eq!(snap.display_offset, 10);
    // 100 lines + the empty prompt line; 5 on screen, 10 kept above.
    assert_eq!(snap.row_text(0), "line 86");
}

#[test]
fn zero_scrollback_keeps_nothing() {
    let mut grid = grid(20, 3, 0);
    lines(&mut grid, 10);
    assert_eq!(grid.history_size(), 0);
}

#[test]
fn the_limit_is_capped() {
    let grid = grid(20, 3, usize::MAX);
    assert_eq!(grid.scrollback_limit(), MAX_SCROLLBACK_LINES);
}

#[test]
fn shrinking_the_limit_drops_old_lines_and_growing_keeps_more() {
    let mut grid = grid(20, 3, 50);
    lines(&mut grid, 60);
    assert_eq!(grid.history_size(), 50);
    grid.set_scrollback(5);
    assert_eq!(grid.history_size(), 5);
    grid.set_scrollback(30);
    lines(&mut grid, 60);
    assert_eq!(grid.history_size(), 30);
}

#[test]
fn scrolling_moves_the_viewport_and_hides_the_cursor() {
    let mut grid = grid(20, 3, 100);
    lines(&mut grid, 10);
    grid.scroll(TerminalScroll::Lines(2));
    let snap = grid.snapshot();
    assert_eq!(snap.display_offset, 2);
    assert_eq!(screen(&snap), ["line 6", "line 7", "line 8"]);
    assert!(!snap.cursor.visible, "the cursor row is below the viewport");
    grid.scroll(TerminalScroll::PageDown);
    assert_eq!(grid.display_offset(), 0);
    grid.scroll(TerminalScroll::PageUp);
    assert_eq!(grid.display_offset(), 3);
    grid.scroll(TerminalScroll::Bottom);
    assert!(grid.snapshot().cursor.visible);
}

#[test]
fn resize_reflows_wrapped_lines() {
    let mut grid = grid(10, 3, 100);
    feed(&mut grid, "0123456789abcde");
    assert_eq!(screen(&grid.snapshot()), ["0123456789", "abcde", ""]);

    let applied = grid.resize(TerminalSize::new(20, 3));
    assert_eq!((applied.width, applied.height), (20, 3));
    assert_eq!(screen(&grid.snapshot())[0], "0123456789abcde");

    // Narrower: the line wraps onto three rows and the cursor follows below, pushing the first
    // two rows into the history.
    grid.resize(TerminalSize::new(5, 3));
    assert_eq!(grid.snapshot().columns, 5);
    assert_eq!(grid.history_size(), 2);
    grid.scroll(TerminalScroll::Top);
    assert_eq!(screen(&grid.snapshot()), ["01234", "56789", "abcde"]);
}

#[test]
fn resize_is_clamped_and_keeps_the_pixel_size() {
    let mut grid = grid(10, 3, 0);
    let applied = grid.resize(TerminalSize::new(0, 0).with_pixels(100, 40));
    assert_eq!((applied.width, applied.height), (2, 1));
    assert_eq!((applied.pixel_width, applied.pixel_height), (100, 40));
    assert_eq!(grid.size(), applied);
    let snap = grid.snapshot();
    assert_eq!((snap.columns, snap.rows, snap.cells.len()), (2, 1, 2));
}

#[test]
fn snapshot_buffers_are_reused() {
    let mut grid = grid(80, 24, 100);
    let mut snap = super::super::TerminalSnapshot::default();
    grid.snapshot_into(&mut snap);
    let cells = snap.cells.as_ptr();
    feed(&mut grid, "more output\r\n");
    grid.snapshot_into(&mut snap);
    assert_eq!(snap.cells.as_ptr(), cells, "same allocation frame to frame");
    assert_eq!(snap.cells.len(), 80 * 24);
}
