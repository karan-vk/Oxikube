//! Which viewport cells a search's matches cover.

use oxikube_ports::TerminalSize;

use super::super::highlight::{HighlightSpan, SearchHighlights, spans_into};
use crate::grid::{GridEvent, GridMatch, GridPoint, TermGrid, TerminalScroll};

fn found(from: (i32, usize), to: (i32, usize)) -> GridMatch {
    GridMatch {
        start: GridPoint::new(from.0, from.1),
        end: GridPoint::new(to.0, to.1),
    }
}

fn span(row: usize, column: usize, cells: usize, current: bool) -> HighlightSpan {
    HighlightSpan {
        row,
        column,
        cells,
        current,
    }
}

fn spans(grid: &mut TermGrid, highlights: &SearchHighlights) -> Vec<HighlightSpan> {
    let mut out = Vec::new();
    spans_into(&mut out, Some(highlights), &grid.snapshot());
    out
}

fn grid(columns: u16, rows: u16) -> TermGrid {
    let mut grid = TermGrid::new(TerminalSize::new(columns, rows), 100);
    let mut events: Vec<GridEvent> = Vec::new();
    grid.advance(b"x", &mut events);
    grid
}

#[test]
fn a_match_is_a_span_in_its_row_and_the_current_one_says_so() {
    let mut grid = grid(10, 3);
    let highlights =
        SearchHighlights::sorted(vec![found((1, 2), (1, 4)), found((0, 0), (0, 1))], Some(1));
    // Sorted into grid order first, so index 1 is the second line's match.
    assert_eq!(
        spans(&mut grid, &highlights),
        [span(0, 0, 2, false), span(1, 2, 3, true)]
    );
}

#[test]
fn a_match_over_several_rows_is_one_span_per_row() {
    let mut grid = grid(10, 4);
    let highlights = SearchHighlights::sorted(vec![found((0, 7), (2, 1))], None);
    assert_eq!(
        spans(&mut grid, &highlights),
        [
            span(0, 7, 3, false),
            span(1, 0, 10, false),
            span(2, 0, 2, false)
        ]
    );
}

#[test]
fn matches_outside_the_viewport_are_left_out_and_scrolling_moves_the_rest() {
    let mut grid = grid(10, 3);
    let mut events: Vec<GridEvent> = Vec::new();
    for line in 0..20 {
        grid.advance(format!("{line}\r\n").as_bytes(), &mut events);
    }
    // Screen lines are 0..3; history is negative.
    let highlights = SearchHighlights::sorted(
        vec![found((-10, 0), (-10, 1)), found((1, 0), (1, 0))],
        Some(0),
    );
    assert_eq!(spans(&mut grid, &highlights), [span(1, 0, 1, false)]);
    grid.scroll(TerminalScroll::Lines(10));
    // Scrolled ten lines up: the history match is on the bottom row, the screen one is gone.
    let rows = grid.snapshot().rows as i32;
    assert_eq!(rows, 3);
    assert_eq!(spans(&mut grid, &highlights), [span(0, 0, 2, true)]);
}

#[test]
fn no_highlights_no_spans() {
    let mut grid = grid(10, 3);
    let mut out = vec![span(0, 0, 1, false)];
    spans_into(&mut out, None, &grid.snapshot());
    assert!(out.is_empty());
}
