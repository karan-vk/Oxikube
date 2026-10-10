//! The wrap-around rule of `n` / `N`, and [`FindNavigator`], the matches of one text.

use std::collections::VecDeque;

/// Sorted match positions: a log's sequence numbers, line numbers, or the starting bytes of the
/// matches in a text.
pub trait MatchList {
    /// How many matches there are.
    fn count(&self) -> usize;
    /// The position of the `index`th match (0 is the first).
    fn at(&self, index: usize) -> Option<u64>;
}

impl MatchList for [u64] {
    fn count(&self) -> usize {
        self.len()
    }

    fn at(&self, index: usize) -> Option<u64> {
        self.get(index).copied()
    }
}

impl MatchList for VecDeque<u64> {
    fn count(&self) -> usize {
        self.len()
    }

    fn at(&self, index: usize) -> Option<u64> {
        self.get(index).copied()
    }
}

/// How many matches are strictly below `position` (so the first match at or after it).
fn below(list: &(impl MatchList + ?Sized), position: u64, inclusive: bool) -> usize {
    let (mut low, mut high) = (0, list.count());
    while low < high {
        let middle = low + (high - low) / 2;
        let at = list.at(middle).unwrap_or(u64::MAX);
        if at < position || (inclusive && at == position) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

/// The match to go to for `n`: the first one after `current` (a match that is gone is skipped by
/// position), else the first at or after `anchor`, wrapping to the first match past the last.
/// `None` when nothing matches.
pub fn next_match(
    list: &(impl MatchList + ?Sized),
    current: Option<u64>,
    anchor: u64,
) -> Option<u64> {
    let index = match current {
        Some(current) => below(list, current, true),
        None => below(list, anchor, false),
    };
    list.at(index).or_else(|| list.at(0))
}

/// The match to go to for `N`: the last one before `current` (else the last match), wrapping to
/// the last past the first. `None` when nothing matches.
pub fn previous_match(list: &(impl MatchList + ?Sized), current: Option<u64>) -> Option<u64> {
    let before = current.and_then(|current| below(list, current, false).checked_sub(1));
    before
        .and_then(|index| list.at(index))
        .or_else(|| list.count().checked_sub(1).and_then(|last| list.at(last)))
}

/// The matches of one text and the one the user is on: what a view keeps for `n` / `N`.
///
/// A view hands it the start of every match of a scan ([`FindNavigator::set`]), asks for [`next`](Self::next) / [`previous`](Self::previous) and draws [`position`](Self::position)
/// as `3 / 12`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindNavigator {
    positions: Vec<u64>,
    current: Option<usize>,
    truncated: bool,
}

impl FindNavigator {
    /// No matches.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the matches (sorted positions). The current match is kept when its position is
    /// still a match; otherwise there is none.
    pub fn set(&mut self, positions: Vec<u64>, truncated: bool) {
        let kept = self
            .current()
            .and_then(|at| positions.binary_search(&at).ok());
        self.positions = positions;
        self.current = kept;
        self.truncated = truncated;
    }

    /// Forgets every match.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// How many matches there are.
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    /// Whether nothing matches.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Whether the scan that made the matches stopped at its cap.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// The position of the match the user is on.
    pub fn current(&self) -> Option<u64> {
        self.current.and_then(|i| self.positions.get(i)).copied()
    }

    /// The index of the current match (0-based).
    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    /// `(n, total)` for "3 / 12": the current match counted from 1.
    pub fn position(&self) -> Option<(usize, usize)> {
        self.current.map(|i| (i + 1, self.positions.len()))
    }

    /// Goes to the next match (see the [module](super) rule) and returns its index; `anchor` is
    /// where to start when no match is current.
    pub fn next(&mut self, anchor: u64) -> Option<usize> {
        let to = next_match(self.positions.as_slice(), self.current(), anchor)?;
        self.go(to)
    }

    /// Goes to the previous match and returns its index.
    pub fn previous(&mut self) -> Option<usize> {
        let to = previous_match(self.positions.as_slice(), self.current())?;
        self.go(to)
    }

    fn go(&mut self, position: u64) -> Option<usize> {
        self.current = self.positions.binary_search(&position).ok();
        self.current
    }
}
