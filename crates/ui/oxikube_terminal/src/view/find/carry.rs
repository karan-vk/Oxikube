//! Keeping the user's place in the matches across a rescan.
//!
//! Grid lines are numbered from the top of the live screen, so every line of output that scrolls
//! the screen moves each match up by one in grid coordinates. The old current match is therefore
//! looked up `shift` lines higher, where `shift` is how much the history grew between the scans.

use crate::grid::{GridMatch, GridPoint};

/// The index in `matches` (top to bottom) of the match the user was on, `old` in the previous
/// scan, after output pushed `shift` lines into the history since.
///
/// An exact hit wins. Otherwise (the line was rewritten, scrolled out of a full scrollback, or
/// the scrollback was already full so the shift is not known) the nearest match at or above where
/// it should be is taken, else the first. `None` only when there are no matches.
pub(super) fn carry_current(old: GridMatch, shift: usize, matches: &[GridMatch]) -> Option<usize> {
    let moved =
        |point: GridPoint| GridPoint::new(point.line.saturating_sub(shift as i32), point.column);
    let (start, end) = (moved(old.start), moved(old.end));
    let at = matches.partition_point(|found| found.start < start);
    if matches
        .get(at)
        .is_some_and(|found| found.start == start && found.end == end)
    {
        return Some(at);
    }
    // `at` is the first match below the wanted place: the one above it is the nearest.
    match at.checked_sub(1) {
        Some(above) => Some(above),
        None => (!matches.is_empty()).then_some(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(line: i32, column: usize) -> GridMatch {
        GridMatch {
            start: GridPoint::new(line, column),
            end: GridPoint::new(line, column + 5),
        }
    }

    #[test]
    fn the_match_is_followed_up_the_screen_by_the_lines_scrolled() {
        let before = [found(2, 0), found(12, 0), found(22, 0)];
        let after = [found(-8, 0), found(2, 0), found(12, 0)];
        assert_eq!(carry_current(before[1], 10, &after), Some(1));
        assert_eq!(carry_current(before[0], 10, &after), Some(0));
    }

    #[test]
    fn without_scrolling_it_stays_put() {
        let matches = [found(2, 0), found(2, 9), found(5, 3)];
        assert_eq!(carry_current(matches[1], 0, &matches), Some(1));
    }

    #[test]
    fn a_match_that_is_gone_falls_back_to_the_nearest_one_above() {
        let after = [found(1, 0), found(8, 0), found(20, 0)];
        assert_eq!(carry_current(found(10, 0), 0, &after), Some(1));
        // Same line, other columns: the one before in reading order.
        assert_eq!(carry_current(found(8, 4), 0, &after), Some(1));
    }

    #[test]
    fn a_match_that_scrolled_off_the_top_falls_back_to_the_first() {
        let after = [found(1, 0), found(8, 0)];
        assert_eq!(carry_current(found(3, 0), 30, &after), Some(0));
    }

    #[test]
    fn no_matches_no_current() {
        assert_eq!(carry_current(found(3, 0), 1, &[]), None);
    }
}
