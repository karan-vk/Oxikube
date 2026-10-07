//! Regex search over the grid, scrollback included: [`GridMatch`], [`GridSearch`] (the scan in
//! bounded slices) and [`TermGrid::search`] (the whole scan at once).

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::{RegexIter, RegexSearch};
use oxikube_domain::{OxiError, OxiResult};

use super::{GridPoint, TermGrid};

/// The most matches one search returns; a pattern like `.` on a full scrollback would otherwise
/// build a list of millions.
pub const MAX_SEARCH_MATCHES: usize = 10_000;

/// Lines one [`GridSearch::step`] scans: about 0.4 ms at 120 columns in release builds (see the
/// `grid_bench` example), so whoever waits for the grid lock behind a search step (the UI thread)
/// waits under the 1 ms budget.
pub const SEARCH_SLICE_LINES: usize = 500;

/// One match: its first and last cell (inclusive), in grid coordinates. A match may span lines,
/// including soft-wrapped ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridMatch {
    /// First cell.
    pub start: GridPoint,
    /// Last cell (inclusive).
    pub end: GridPoint,
}

/// A search of the whole grid in slices of about [`SEARCH_SLICE_LINES`] lines: call
/// [`step`](Self::step) under the grid lock, release the lock, repeat until it returns `true`.
/// Each slice ends where a line ends without wrapping, so a match (which never crosses a hard line
/// break) is never cut in two.
///
/// Between steps the grid must not receive output (it would move the lines already searched; the
/// bridge holds the pump off for the whole search). Scrolling and selecting are fine. A resize or
/// a smaller scrollback in between makes the next step start over.
pub struct GridSearch {
    /// `None`: the empty pattern, which matches nothing.
    regex: Option<RegexSearch>,
    slice_lines: usize,
    /// The layout the scan so far was made against; `None` before the first step.
    layout: Option<u64>,
    next_line: i32,
    done: bool,
    matches: Vec<GridMatch>,
    /// Lines of history when the last step ran (see [`history_size`](Self::history_size)).
    history_size: usize,
}

impl std::fmt::Debug for GridSearch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the pattern or the matches' text: the user may be searching for a secret.
        f.debug_struct("GridSearch")
            .field("next_line", &self.next_line)
            .field("done", &self.done)
            .field("matches", &self.matches.len())
            .finish_non_exhaustive()
    }
}

impl GridSearch {
    /// A search for `pattern` (a regex). Smart case: it ignores case unless the pattern has an
    /// upper-case letter. An empty pattern matches nothing.
    ///
    /// # Errors
    ///
    /// `Validation` when `pattern` is not a valid regex (or compiles to something too large).
    pub fn new(pattern: &str) -> OxiResult<Self> {
        Self::with_slice(pattern, SEARCH_SLICE_LINES)
    }

    /// [`new`](Self::new) scanning `slice_lines` lines per step (at least one).
    pub(crate) fn with_slice(pattern: &str, slice_lines: usize) -> OxiResult<Self> {
        let regex =
            if pattern.is_empty() {
                None
            } else {
                Some(RegexSearch::new(pattern).map_err(|err| {
                    OxiError::validation(format!("invalid search pattern: {err}"))
                })?)
            };
        Ok(Self {
            done: regex.is_none(),
            regex,
            slice_lines: slice_lines.max(1),
            layout: None,
            next_line: 0,
            matches: Vec::new(),
            history_size: 0,
        })
    }

    /// Scans the next slice of `grid`, top to bottom; `true` once the search is complete (the
    /// bottom of the screen reached, or [`MAX_SEARCH_MATCHES`] found).
    pub fn step(&mut self, grid: &TermGrid) -> bool {
        let Some(regex) = self.regex.as_mut() else {
            return true;
        };
        if self.layout != Some(grid.layout_generation) {
            // First step, or the lines moved since the last one: from the top.
            self.layout = Some(grid.layout_generation);
            self.matches.clear();
            self.next_line = grid.term.topmost_line().0;
            self.done = false;
        }
        if self.done {
            return true;
        }
        let term = &grid.term;
        self.history_size = grid.history_size();
        let bottom = term.bottommost_line().0;
        let last_column = term.last_column();
        let first = self.next_line.max(term.topmost_line().0);
        let mut last = first
            .saturating_add(self.slice_lines as i32 - 1)
            .min(bottom);
        // Never end a slice inside a soft-wrapped line.
        while last < bottom
            && term.grid()[Line(last)][last_column]
                .flags
                .contains(Flags::WRAPLINE)
        {
            last += 1;
        }
        let start = Point::new(Line(first), Column(0));
        let end = Point::new(Line(last), last_column);
        let room = MAX_SEARCH_MATCHES - self.matches.len();
        self.matches.extend(
            RegexIter::new(start, end, Direction::Right, term, regex)
                .take(room)
                .map(|found| GridMatch {
                    start: (*found.start()).into(),
                    end: (*found.end()).into(),
                }),
        );
        self.next_line = last + 1;
        self.done = last >= bottom || self.matches.len() >= MAX_SEARCH_MATCHES;
        self.done
    }

    /// Lines of history the grid had when the last step ran. While no output arrives, the lines
    /// the matches are on move up by how much this grows: a caller that keeps a match across two
    /// searches shifts it by the difference.
    pub fn history_size(&self) -> usize {
        self.history_size
    }

    /// The matches found, top to bottom (all of them once [`step`](Self::step) returned `true`).
    pub fn into_matches(self) -> Vec<GridMatch> {
        self.matches
    }
}

impl TermGrid {
    /// Every match of `pattern` (a regex) from the oldest history line to the bottom of the
    /// screen, top to bottom, at most [`MAX_SEARCH_MATCHES`], in one go. Smart case: the search
    /// ignores case unless the pattern has an upper-case letter. An empty pattern matches nothing.
    /// A caller that shares the grid with the UI thread runs a [`GridSearch`] instead.
    ///
    /// Soft-wrapped lines are searched as one line, so a match can continue on the next row.
    ///
    /// # Errors
    ///
    /// `Validation` when `pattern` is not a valid regex (or compiles to something too large).
    pub fn search(&self, pattern: &str) -> OxiResult<Vec<GridMatch>> {
        let mut search = GridSearch::new(pattern)?;
        while !search.step(self) {}
        Ok(search.into_matches())
    }

    /// Scrolls the view so `point` is visible (a search match, say); nothing when it already is.
    pub fn scroll_to(&mut self, point: GridPoint) {
        let point = self.clamp(point);
        self.term.scroll_to_point(point);
        self.listener.discard();
    }
}
