//! [`RowLayout`]: what one viewport row paints, worked out from the snapshot without GPUI.
//!
//! A row becomes three lists, in cell coordinates:
//!
//! * **backgrounds**: adjacent cells with the same non-default background merged into one span;
//! * **runs**: adjacent cells with the same text style (colour, bold, italic) batched into one
//!   string to shape. Blank cells inside a run stay in it, blanks at its ends are dropped. A wide
//!   glyph is a run of its own (the element shapes runs with every glyph forced to one cell), and
//!   its spacer cell is skipped;
//! * **decorations**: underline (single, double, curly, dotted, dashed) and strikethrough spans.
//!
//! Combining marks follow their base character in the run's text. Hidden cells draw no text.

use std::ops::Range;

use gpui::Hsla;

use super::palette::TerminalPalette;
use crate::grid::{CellFlags, SnapshotCell, TerminalSnapshot};

/// The cells that only hold space for a wide glyph.
pub(super) const SPACERS: CellFlags =
    CellFlags::WIDE_CHAR_SPACER.union(CellFlags::LEADING_WIDE_CHAR_SPACER);

/// A background colour over `cells` cells from `column`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundSpan {
    /// First column.
    pub column: usize,
    /// Number of cells.
    pub cells: usize,
    /// The colour.
    pub color: Hsla,
}

/// One shaped run: text in one style starting at `column`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    /// The column of the first character.
    pub column: usize,
    /// Cells the run covers (a wide glyph covers two).
    pub cells: usize,
    /// The run's text in [`RowLayout::text`].
    pub bytes: Range<usize>,
    /// Text colour.
    pub color: Hsla,
    /// Bold weight.
    pub bold: bool,
    /// Italic style.
    pub italic: bool,
    /// A single double-width glyph.
    pub wide: bool,
}

/// How a decoration is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecorationKind {
    /// One straight line under the text.
    Underline,
    /// Two straight lines.
    DoubleUnderline,
    /// A wavy line.
    CurlyUnderline,
    /// A dotted line.
    DottedUnderline,
    /// A dashed line.
    DashedUnderline,
    /// A line through the middle.
    Strikethrough,
}

/// A decoration over `cells` cells from `column`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecorationSpan {
    /// First column.
    pub column: usize,
    /// Number of cells.
    pub cells: usize,
    /// What to draw.
    pub kind: DecorationKind,
    /// Its colour (the text colour).
    pub color: Hsla,
}

/// What one row paints. Buffers are reused when a row is laid out again.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowLayout {
    /// The text of every run, back to back.
    pub text: String,
    /// Background spans, left to right.
    pub backgrounds: Vec<BackgroundSpan>,
    /// Text runs, left to right.
    pub runs: Vec<TextSpan>,
    /// Decoration spans, left to right per kind.
    pub decorations: Vec<DecorationSpan>,
}

/// The run being built and the blanks seen after it.
struct OpenRun {
    span: TextSpan,
    blanks: usize,
}

impl RowLayout {
    /// Lays out viewport `row` of `snapshot` with `palette`, replacing what was here.
    pub fn build(&mut self, snapshot: &TerminalSnapshot, row: usize, palette: &TerminalPalette) {
        self.text.clear();
        self.backgrounds.clear();
        self.runs.clear();
        self.decorations.clear();
        let start = row * snapshot.columns;
        let Some(cells) = snapshot.cells.get(start..start + snapshot.columns) else {
            return;
        };
        let mut marks = snapshot
            .zerowidth
            .partition_point(|&(index, _)| index < start);
        let mut open: Option<OpenRun> = None;
        for (column, cell) in cells.iter().enumerate() {
            let index = start + column;
            let (fg, bg) = palette.cell_colors(cell);
            if let Some(bg) = bg {
                self.push_background(column, bg);
            }
            if !cell.flags.contains(CellFlags::HIDDEN) {
                self.push_decorations(column, cell.flags, fg);
            }
            if cell.flags.intersects(SPACERS) {
                continue;
            }
            let first_mark = marks;
            while snapshot
                .zerowidth
                .get(marks)
                .is_some_and(|&(at, _)| at == index)
            {
                marks += 1;
            }
            let blank = is_blank(cell) && first_mark == marks;
            if blank || cell.flags.contains(CellFlags::HIDDEN) {
                if let Some(run) = open.as_mut() {
                    run.blanks += 1;
                }
                continue;
            }
            let wide = cell.flags.contains(CellFlags::WIDE_CHAR);
            let bold = cell.flags.contains(CellFlags::BOLD);
            let italic = cell.flags.contains(CellFlags::ITALIC);
            // An open run is never a wide glyph: those are closed as soon as they are written.
            let continues = open.as_ref().is_some_and(|run| {
                !wide
                    && run.span.column + run.span.cells + run.blanks == column
                    && (run.span.color, run.span.bold, run.span.italic) == (fg, bold, italic)
            });
            if continues {
                let run = open.as_mut().expect("checked above");
                for _ in 0..run.blanks {
                    self.text.push(' ');
                }
                run.span.cells += run.blanks + 1;
                run.blanks = 0;
            } else {
                self.close(open.take());
                open = Some(OpenRun {
                    span: TextSpan {
                        column,
                        cells: if wide { 2 } else { 1 },
                        bytes: self.text.len()..self.text.len(),
                        color: fg,
                        bold,
                        italic,
                        wide,
                    },
                    blanks: 0,
                });
            }
            self.text.push(cell.c);
            for &(_, mark) in &snapshot.zerowidth[first_mark..marks] {
                self.text.push(mark);
            }
            if wide {
                // A wide glyph is a run of its own: runs force every glyph to one cell.
                self.close(open.take());
            }
        }
        self.close(open);
    }

    /// Ends `run` at the text written so far (trailing blanks were never written).
    fn close(&mut self, run: Option<OpenRun>) {
        if let Some(mut run) = run {
            run.span.bytes.end = self.text.len();
            self.runs.push(run.span);
        }
    }

    fn push_background(&mut self, column: usize, color: Hsla) {
        if let Some(last) = self.backgrounds.last_mut()
            && last.color == color
            && last.column + last.cells == column
        {
            last.cells += 1;
            return;
        }
        self.backgrounds.push(BackgroundSpan {
            column,
            cells: 1,
            color,
        });
    }

    fn push_decorations(&mut self, column: usize, flags: CellFlags, color: Hsla) {
        let underline = if flags.contains(CellFlags::UNDERCURL) {
            Some(DecorationKind::CurlyUnderline)
        } else if flags.contains(CellFlags::DOUBLE_UNDERLINE) {
            Some(DecorationKind::DoubleUnderline)
        } else if flags.contains(CellFlags::DOTTED_UNDERLINE) {
            Some(DecorationKind::DottedUnderline)
        } else if flags.contains(CellFlags::DASHED_UNDERLINE) {
            Some(DecorationKind::DashedUnderline)
        } else if flags.contains(CellFlags::UNDERLINE) {
            Some(DecorationKind::Underline)
        } else {
            None
        };
        let strike = flags
            .contains(CellFlags::STRIKEOUT)
            .then_some(DecorationKind::Strikethrough);
        for kind in [underline, strike].into_iter().flatten() {
            let merged = self.decorations.iter_mut().rev().find(|span| {
                span.kind == kind && span.color == color && span.column + span.cells == column
            });
            match merged {
                Some(span) => span.cells += 1,
                None => self.decorations.push(DecorationSpan {
                    column,
                    cells: 1,
                    kind,
                    color,
                }),
            }
        }
    }
}

/// Whether a cell shows nothing but its background.
fn is_blank(cell: &SnapshotCell) -> bool {
    matches!(cell.c, ' ' | '\t' | '\0')
}

/// The columns of viewport `row` the snapshot's selection covers, if any.
pub fn selected_columns(snapshot: &TerminalSnapshot, row: usize) -> Option<Range<usize>> {
    let (start, end, block) = snapshot.selection?;
    if snapshot.columns == 0 {
        return None;
    }
    let line = row as i32 - snapshot.display_offset as i32;
    if line < start.line || line > end.line {
        return None;
    }
    let last = snapshot.columns - 1;
    let (first, final_column) = if block {
        (
            start.column.min(end.column),
            start.column.max(end.column).min(last),
        )
    } else {
        (
            if line == start.line { start.column } else { 0 },
            if line == end.line {
                end.column.min(last)
            } else {
                last
            },
        )
    };
    (first <= final_column).then_some(first..final_column + 1)
}
