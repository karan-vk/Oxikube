//! Regex search over screen and scrollback.

use oxikube_domain::ErrorKind;

use super::super::{GridMatch, GridPoint, TerminalScroll};
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
