//! Regex search over the grid, scrollback included: [`GridMatch`] and [`TermGrid::search`].

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Point};
use alacritty_terminal::term::search::{RegexIter, RegexSearch};
use oxikube_domain::{OxiError, OxiResult};

use super::{GridPoint, TermGrid};

/// The most matches one [`TermGrid::search`] returns; a pattern like `.` on a full scrollback
/// would otherwise build a list of millions.
pub const MAX_SEARCH_MATCHES: usize = 10_000;

/// One match: its first and last cell (inclusive), in grid coordinates. A match may span lines,
/// including soft-wrapped ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridMatch {
    /// First cell.
    pub start: GridPoint,
    /// Last cell (inclusive).
    pub end: GridPoint,
}

impl TermGrid {
    /// Every match of `pattern` (a regex) from the oldest history line to the bottom of the
    /// screen, top to bottom, at most [`MAX_SEARCH_MATCHES`]. Smart case: the search ignores case
    /// unless the pattern has an upper-case letter. An empty pattern matches nothing.
    ///
    /// Soft-wrapped lines are searched as one line, so a match can continue on the next row.
    ///
    /// # Errors
    ///
    /// `Validation` when `pattern` is not a valid regex (or compiles to something too large).
    pub fn search(&self, pattern: &str) -> OxiResult<Vec<GridMatch>> {
        if pattern.is_empty() {
            return Ok(Vec::new());
        }
        let mut regex = RegexSearch::new(pattern)
            .map_err(|err| OxiError::validation(format!("invalid search pattern: {err}")))?;
        let start = Point::new(self.term.topmost_line(), Column(0));
        let end = Point::new(self.term.bottommost_line(), self.term.last_column());
        let matches = RegexIter::new(start, end, Direction::Right, &self.term, &mut regex)
            .take(MAX_SEARCH_MATCHES)
            .map(|found| GridMatch {
                start: (*found.start()).into(),
                end: (*found.end()).into(),
            })
            .collect();
        Ok(matches)
    }

    /// Scrolls the view so `point` is visible (a search match, say); nothing when it already is.
    pub fn scroll_to(&mut self, point: GridPoint) {
        let point = self.clamp(point);
        self.term.scroll_to_point(point);
        self.listener.discard();
    }
}
