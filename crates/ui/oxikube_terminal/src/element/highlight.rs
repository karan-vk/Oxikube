//! Search matches painted over the cells (E09-S11): which viewport cells a search's matches cover.
//!
//! The host passes the matches as grid points ([`SearchHighlights`]); each frame this turns the
//! ones inside the viewport into row spans. Pure, so it is table-tested without a window.

use std::rc::Rc;

use crate::grid::{GridMatch, TerminalSnapshot};

/// The matches of a search and the one the user is on, for [`TerminalElement::highlights`](crate::TerminalElement::highlights)
/// (`crate::TerminalElement::highlights`).
#[derive(Debug, Clone, Default)]
pub struct SearchHighlights {
    /// Every match, in grid order (as [`TerminalState::search`](crate::TerminalState::search)
    /// returns them, sorted by [`sorted`](Self::sorted)).
    pub matches: Rc<Vec<GridMatch>>,
    /// Index into `matches` of the current match.
    pub current: Option<usize>,
}

impl SearchHighlights {
    /// Highlights over `matches` sorted into grid order, with `current` as given.
    pub fn sorted(mut matches: Vec<GridMatch>, current: Option<usize>) -> Self {
        matches.sort_by_key(|found| found.start);
        Self {
            matches: Rc::new(matches),
            current,
        }
    }
}

/// A run of highlighted cells in one viewport row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HighlightSpan {
    pub(super) row: usize,
    pub(super) column: usize,
    pub(super) cells: usize,
    /// The match the user is on.
    pub(super) current: bool,
}

/// Fills `out` with the spans of the matches inside `snapshot`'s viewport. `matches` must be in
/// grid order (see [`SearchHighlights::sorted`]); a match may cover several rows.
pub(super) fn spans_into(
    out: &mut Vec<HighlightSpan>,
    highlights: Option<&SearchHighlights>,
    snapshot: &TerminalSnapshot,
) {
    out.clear();
    let Some(highlights) = highlights else {
        return;
    };
    if snapshot.rows == 0 || snapshot.columns == 0 {
        return;
    }
    let offset = snapshot.display_offset as i32;
    let first_line = -offset;
    let last_line = snapshot.rows as i32 - 1 - offset;
    let matches = &highlights.matches;
    // Matches never overlap, so their ends are in order too.
    let first = matches.partition_point(|found| found.end.line < first_line);
    let last_column = snapshot.columns - 1;
    for (index, found) in matches.iter().enumerate().skip(first) {
        if found.start.line > last_line {
            break;
        }
        let lines = found.start.line.max(first_line)..=found.end.line.min(last_line);
        for line in lines {
            let from = if line == found.start.line {
                found.start.column
            } else {
                0
            };
            let to = if line == found.end.line {
                found.end.column
            } else {
                last_column
            };
            let (from, to) = (from.min(last_column), to.min(last_column));
            if to < from {
                continue;
            }
            out.push(HighlightSpan {
                row: (line + offset) as usize,
                column: from,
                cells: to + 1 - from,
                current: highlights.current == Some(index),
            });
        }
    }
}
