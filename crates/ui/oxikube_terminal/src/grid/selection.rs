//! Selection over the grid: [`GridPoint`], [`SelectionKind`] and the [`TermGrid`] selection API.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};

use super::{LineDamage, TermGrid};

/// A selection as a snapshot carries it: first and last cell (inclusive, start before end) and
/// whether it is a block.
pub(super) type SelectionRange = (GridPoint, GridPoint, bool);

/// A cell of the whole grid, scrollback included: `line` `0` is the top row of the live screen,
/// negative lines are history (`-1` the newest history line), `column` counts from `0`.
///
/// Grid points stay put while the view scrolls but move as output pushes lines into the history;
/// convert from what is on screen with [`from_viewport`](Self::from_viewport).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GridPoint {
    /// Line (negative: scrollback).
    pub line: i32,
    /// Column.
    pub column: usize,
}

impl GridPoint {
    /// A grid point.
    pub fn new(line: i32, column: usize) -> Self {
        Self { line, column }
    }

    /// The grid point under viewport `row`, `column` while the view is scrolled `display_offset`
    /// lines up.
    pub fn from_viewport(row: usize, column: usize, display_offset: usize) -> Self {
        Self {
            line: row as i32 - display_offset as i32,
            column,
        }
    }
}

impl From<Point> for GridPoint {
    fn from(point: Point) -> Self {
        Self {
            line: point.line.0,
            column: point.column.0,
        }
    }
}

/// What a selection grows by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionKind {
    /// Cell by cell (a plain drag).
    Cell,
    /// Whole words (double click): grows to the nearest separator on each side.
    Word,
    /// Whole lines (triple click).
    Line,
    /// A rectangle (alt-drag).
    Block,
}

/// Which half of a cell a pointer is over: a selection that starts on the right half of a cell
/// does not include it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SelectionSide {
    /// The left half.
    #[default]
    Left,
    /// The right half.
    Right,
}

impl From<SelectionSide> for Side {
    fn from(side: SelectionSide) -> Self {
        match side {
            SelectionSide::Left => Side::Left,
            SelectionSide::Right => Side::Right,
        }
    }
}

impl TermGrid {
    /// Starts a selection of `kind` at `point` (replacing any selection).
    pub fn start_selection(&mut self, kind: SelectionKind, point: GridPoint, side: SelectionSide) {
        let ty = match kind {
            SelectionKind::Cell => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
            SelectionKind::Block => SelectionType::Block,
        };
        let point = self.clamp(point);
        self.term.selection = Some(Selection::new(ty, point, side.into()));
    }

    /// Moves the end of the selection to `point`; nothing without a selection.
    pub fn update_selection(&mut self, point: GridPoint, side: SelectionSide) {
        let point = self.clamp(point);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side.into());
        }
    }

    /// Drops the selection.
    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// The selected text, `None` without a (non-empty) selection. Soft-wrapped lines are joined
    /// without a newline, wide glyphs appear once, trailing blanks of each line are dropped.
    pub fn selection_text(&self) -> Option<String> {
        self.term
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    /// `point` moved inside the grid (oldest history line to last screen line, first to last
    /// column).
    pub(super) fn clamp(&self, point: GridPoint) -> Point {
        let top = self.term.topmost_line().0;
        let bottom = self.term.bottommost_line().0;
        let last_column = self.term.last_column().0;
        Point::new(
            Line(point.line.clamp(top, bottom)),
            Column(point.column.min(last_column)),
        )
    }
}

/// Adds the viewport rows `range` covers (scrolled `display_offset` lines up, `rows` x `columns`
/// viewport) to `damage` as whole rows, keeping it sorted by row with one entry per row.
///
/// alacritty's damage leaves the selection out (its renderer diffs selections itself), so a
/// selection that appears, grows, shrinks or goes away changes rows nothing else reports.
pub(super) fn damage_selection(
    damage: &mut Vec<LineDamage>,
    range: SelectionRange,
    display_offset: usize,
    rows: usize,
    columns: usize,
) {
    let offset = display_offset as i32;
    let first = range.0.line + offset;
    let last = range.1.line + offset;
    if rows == 0 || columns == 0 || last < 0 || first >= rows as i32 {
        return;
    }
    let first = first.max(0) as usize;
    let last = (last as usize).min(rows - 1);
    damage.extend((first..=last).map(|row| LineDamage {
        row,
        left: 0,
        right: columns - 1,
    }));
    damage.sort_unstable_by_key(|line| line.row);
    damage.dedup_by(|later, kept| {
        if later.row != kept.row {
            return false;
        }
        kept.left = kept.left.min(later.left);
        kept.right = kept.right.max(later.right);
        true
    });
}
