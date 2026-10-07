//! Regex search over screen and scrollback.

use oxikube_domain::ErrorKind;

use oxikube_ports::TerminalSize;

use super::super::{GridMatch, GridPoint, GridSearch, TermGrid, TerminalScroll};
use super::{feed, grid};

fn span(start: (i32, usize), end: (i32, usize)) -> GridMatch {
    GridMatch {
        start: GridPoint::new(start.0, start.1),
        end: GridPoint::new(end.0, end.1),
    }
}

#[test]
fn finds_every_match_top_to_bottom() {
    let mut grid = grid(20, 4, 0);
    feed(&mut grid, "pod-a Running\r\npod-b Pending\r\npod-c Running");
    let matches = grid.search("Running").unwrap();
    assert_eq!(matches, [span((0, 6), (0, 12)), span((2, 6), (2, 12))]);
}

#[test]
fn finds_matches_across_soft_wrapped_lines() {
    let mut grid = grid(6, 3, 0);
    feed(&mut grid, "xxxxkubectl");
    let matches = grid.search("kubectl").unwrap();
    assert_eq!(matches, [span((0, 4), (1, 4))]);
}

#[test]
fn searches_the_scrollback() {
    let mut grid = grid(20, 2, 100);
    feed(&mut grid, "needle one\r\nhay\r\nhay\r\nneedle two");
    let matches = grid.search("needle \\w+").unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].start, GridPoint::new(-2, 0));
    assert_eq!(matches[1], span((1, 0), (1, 9)));

    grid.scroll_to(matches[0].start);
    assert_eq!(grid.display_offset(), 2);
    grid.scroll(TerminalScroll::Bottom);
    grid.scroll_to(matches[1].start);
    assert_eq!(grid.display_offset(), 0, "already visible: no scroll");
}

#[test]
fn smart_case() {
    let mut grid = grid(20, 2, 0);
    feed(&mut grid, "Error error ERROR");
    assert_eq!(grid.search("error").unwrap().len(), 3);
    assert_eq!(grid.search("Error").unwrap().len(), 1);
}

#[test]
fn empty_and_invalid_patterns() {
    let mut grid = grid(20, 2, 0);
    feed(&mut grid, "text");
    assert!(grid.search("").unwrap().is_empty());
    assert!(grid.search("nothing").unwrap().is_empty());
    let err = grid.search("(unclosed").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}

/// Runs `search` to the end, one step at a time; returns the matches and the steps taken.
fn run(mut search: GridSearch, grid: &TermGrid) -> (Vec<GridMatch>, usize) {
    let mut steps = 1;
    while !search.step(grid) {
        steps += 1;
    }
    (search.into_matches(), steps)
}

#[test]
fn a_search_scans_in_bounded_slices() {
    let mut grid = grid(20, 4, 100);
    for line in 0..40 {
        feed(&mut grid, format!("line {line}\r\n"));
    }
    let whole = grid.search("line 1\\d").unwrap();
    assert_eq!(whole.len(), 10);
    // 44 lines (40 history + 4 screen) in slices of 10: five steps, the lock released between.
    let (sliced, steps) = run(GridSearch::with_slice("line 1\\d", 10).unwrap(), &grid);
    assert_eq!(steps, 5);
    assert_eq!(sliced, whole);
}

#[test]
fn a_slice_never_ends_inside_a_soft_wrapped_line() {
    let mut grid = grid(6, 3, 100);
    // History line -1 is `a`; line 0 is `xxxxku`, soft-wrapping onto `bectl`. A two-line slice
    // from the top would end at line 0 and cut the match.
    feed(&mut grid, "a\r\nxxxxkubectl\r\nb");
    let (matches, steps) = run(GridSearch::with_slice("kubectl", 2).unwrap(), &grid);
    assert_eq!(matches, [span((0, 4), (1, 4))]);
    assert_eq!(steps, 2, "lines -1..=1, then line 2");
}

#[test]
fn a_resize_between_slices_starts_the_search_over() {
    let mut grid = grid(20, 2, 100);
    feed(&mut grid, "needle one\r\nhay\r\nhay\r\nneedle two");
    let mut search = GridSearch::with_slice("needle", 1).unwrap();
    assert!(!search.step(&grid));
    // Narrower: the lines reflow, every grid point moves.
    grid.resize(TerminalSize::new(4, 2));
    let (matches, _) = run(search, &grid);
    assert_eq!(matches, grid.search("needle").unwrap());
}

#[test]
fn an_empty_pattern_is_done_at_once() {
    let grid = grid(20, 2, 0);
    let mut search = GridSearch::new("").unwrap();
    assert!(search.step(&grid));
    assert!(search.into_matches().is_empty());
}
