//! Selection kinds and the selected text.

use super::super::{GridPoint, SelectionKind, SelectionSide};
use super::{feed, grid};

const LEFT: SelectionSide = SelectionSide::Left;
const RIGHT: SelectionSide = SelectionSide::Right;

#[test]
fn cell_selection_within_a_line() {
    let mut grid = grid(20, 3, 0);
    feed(&mut grid, "hello world");
    grid.start_selection(SelectionKind::Cell, GridPoint::new(0, 6), LEFT);
    grid.update_selection(GridPoint::new(0, 10), RIGHT);
    assert_eq!(grid.selection_text().as_deref(), Some("world"));
    let snap = grid.snapshot();
    assert!(snap.is_selected(0, 6) && snap.is_selected(0, 10));
    assert!(!snap.is_selected(0, 5));
    grid.clear_selection();
    assert_eq!(grid.selection_text(), None);
    assert!(grid.snapshot().selection.is_none());
}

#[test]
fn selection_across_a_soft_wrap_has_no_newline() {
    let mut grid = grid(5, 3, 0);
    feed(&mut grid, "abcdefgh\r\nxy");
    grid.start_selection(SelectionKind::Cell, GridPoint::new(0, 2), LEFT);
    grid.update_selection(GridPoint::new(1, 1), RIGHT);
    assert_eq!(grid.selection_text().as_deref(), Some("cdefg"));
    // Across the hard line break the newline is kept.
    grid.update_selection(GridPoint::new(2, 1), RIGHT);
    assert_eq!(grid.selection_text().as_deref(), Some("cdefgh\nxy"));
}

#[test]
fn word_selection_grows_to_separators() {
    let mut grid = grid(30, 2, 0);
    feed(&mut grid, "kubectl get pods,nodes");
    grid.start_selection(SelectionKind::Word, GridPoint::new(0, 13), LEFT);
    assert_eq!(grid.selection_text().as_deref(), Some("pods"));
}

#[test]
fn line_selection_takes_whole_lines() {
    let mut grid = grid(20, 3, 0);
    feed(&mut grid, "first\r\nsecond\r\nthird");
    grid.start_selection(SelectionKind::Line, GridPoint::new(1, 3), LEFT);
    assert_eq!(grid.selection_text().as_deref(), Some("second\n"));
}

#[test]
fn block_selection_is_a_rectangle() {
    let mut grid = grid(20, 3, 0);
    feed(&mut grid, "abcdef\r\nghijkl\r\nmnopqr");
    grid.start_selection(SelectionKind::Block, GridPoint::new(0, 1), LEFT);
    grid.update_selection(GridPoint::new(2, 3), RIGHT);
    assert_eq!(grid.selection_text().as_deref(), Some("bcd\nhij\nnop"));
    let snap = grid.snapshot();
    assert!(snap.is_selected(1, 2));
    assert!(!snap.is_selected(1, 4));
}

#[test]
fn selection_reaches_into_the_scrollback() {
    let mut grid = grid(10, 2, 100);
    feed(&mut grid, "old\r\nmiddle\r\nnew\r\n");
    // "old" and "middle" scrolled into history (lines -2 and -1).
    grid.start_selection(SelectionKind::Cell, GridPoint::new(-2, 0), LEFT);
    grid.update_selection(GridPoint::new(0, 2), RIGHT);
    assert_eq!(grid.selection_text().as_deref(), Some("old\nmiddle\nnew"));
}

#[test]
fn points_outside_the_grid_are_clamped() {
    let mut grid = grid(5, 2, 0);
    feed(&mut grid, "abc\r\ndef");
    grid.start_selection(SelectionKind::Cell, GridPoint::new(-50, 0), LEFT);
    grid.update_selection(GridPoint::new(50, 99), RIGHT);
    assert_eq!(grid.selection_text().as_deref(), Some("abc\ndef"));
}

#[test]
fn a_selection_in_the_scrolled_view_maps_viewport_rows() {
    let point = GridPoint::from_viewport(0, 4, 3);
    assert_eq!(point, GridPoint::new(-3, 4));
}
