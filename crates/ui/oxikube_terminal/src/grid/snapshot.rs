//! [`TerminalSnapshot`]: everything the element needs to paint one frame, copied out of the grid
//! under its lock so painting never holds it.

use std::sync::Arc;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::TermDamage;
use alacritty_terminal::term::color::COUNT as COLOR_COUNT;
use bitflags::bitflags;

use super::{GridPoint, TermGrid, convert};

/// An RGB colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TermRgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

/// A cell colour as the process chose it; the element resolves palette entries against the
/// theme (and [`TerminalSnapshot::color_overrides`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TermColor {
    /// The default foreground.
    Foreground,
    /// The default background.
    Background,
    /// The cursor colour.
    Cursor,
    /// The bright variant of the default foreground.
    BrightForeground,
    /// The dim variant of the default foreground.
    DimForeground,
    /// A palette entry: `0..16` the ANSI colours (`8..16` bright), `16..232` the colour cube,
    /// `232..256` the grey ramp.
    Indexed(u8),
    /// The dim variant of ANSI colour `0..8`.
    Dim(u8),
    /// A direct (24-bit) colour.
    Rgb(TermRgb),
}

bitflags! {
    /// Cell attributes.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct CellFlags: u16 {
        /// Bold (SGR 1).
        const BOLD = 1;
        /// Italic (SGR 3).
        const ITALIC = 1 << 1;
        /// Single underline (SGR 4).
        const UNDERLINE = 1 << 2;
        /// Double underline (SGR 21 / 4:2).
        const DOUBLE_UNDERLINE = 1 << 3;
        /// Curly underline (SGR 4:3).
        const UNDERCURL = 1 << 4;
        /// Dotted underline (SGR 4:4).
        const DOTTED_UNDERLINE = 1 << 5;
        /// Dashed underline (SGR 4:5).
        const DASHED_UNDERLINE = 1 << 6;
        /// Foreground and background swapped (SGR 7).
        const INVERSE = 1 << 7;
        /// Faint (SGR 2).
        const DIM = 1 << 8;
        /// Invisible (SGR 8).
        const HIDDEN = 1 << 9;
        /// Crossed out (SGR 9).
        const STRIKEOUT = 1 << 10;
        /// The first cell of a double-width glyph.
        const WIDE_CHAR = 1 << 11;
        /// The second cell of a double-width glyph: paint nothing.
        const WIDE_CHAR_SPACER = 1 << 12;
        /// Last-column placeholder for a wide glyph that wrapped to the next line: paint nothing.
        const LEADING_WIDE_CHAR_SPACER = 1 << 13;
        /// The line continues on the next one (soft wrap).
        const WRAPLINE = 1 << 14;
    }
}

bitflags! {
    /// Terminal modes the process switched on; the keymap (E09-S06) and the element read them.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct TerminalModes: u32 {
        /// The cursor is shown (DECTCEM).
        const SHOW_CURSOR = 1;
        /// Cursor keys send application sequences (DECCKM).
        const APP_CURSOR = 1 << 1;
        /// The keypad sends application sequences (DECKPAM).
        const APP_KEYPAD = 1 << 2;
        /// Pastes are wrapped in `ESC [200~` / `ESC [201~`.
        const BRACKETED_PASTE = 1 << 3;
        /// Focus changes are reported (`ESC [I` / `ESC [O`).
        const FOCUS_IN_OUT = 1 << 4;
        /// The alternate screen is active (no scrollback).
        const ALT_SCREEN = 1 << 5;
        /// Text wraps at the right margin (DECAWM).
        const LINE_WRAP = 1 << 6;
        /// The wheel sends cursor keys on the alternate screen.
        const ALTERNATE_SCROLL = 1 << 7;
        /// Mouse clicks are reported (1000).
        const MOUSE_REPORT_CLICK = 1 << 8;
        /// Mouse drags are reported (1002).
        const MOUSE_DRAG = 1 << 9;
        /// All mouse motion is reported (1003).
        const MOUSE_MOTION = 1 << 10;
        /// Mouse reports use the SGR encoding (1006).
        const SGR_MOUSE = 1 << 11;
        /// Mouse reports use the UTF-8 encoding (1005).
        const UTF8_MOUSE = 1 << 12;
        /// Line feed also returns the carriage (LNM).
        const LINE_FEED_NEW_LINE = 1 << 13;
        /// Some kitty keyboard protocol flag is on.
        const KITTY_KEYBOARD = 1 << 14;
        /// Any mouse reporting mode.
        const MOUSE_MODE = Self::MOUSE_REPORT_CLICK.bits()
            | Self::MOUSE_DRAG.bits()
            | Self::MOUSE_MOTION.bits();
    }
}

/// How the cursor is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    /// A filled block.
    #[default]
    Block,
    /// An underline.
    Underline,
    /// A vertical bar.
    Beam,
    /// An outlined block (the unfocused look).
    HollowBlock,
}

/// The cursor of a snapshot, in viewport coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerminalCursor {
    /// Viewport row (`0` = top of what is shown).
    pub row: usize,
    /// Column.
    pub column: usize,
    /// Shape the process asked for (DECSCUSR).
    pub shape: CursorShape,
    /// Whether to draw it: the process did not hide it and it is inside the viewport (it is not
    /// when the view is scrolled up past it).
    pub visible: bool,
}

/// One cell of the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotCell {
    /// The character (a space for an empty cell). Combining marks are in
    /// [`TerminalSnapshot::zerowidth`].
    pub c: char,
    /// Foreground colour.
    pub fg: TermColor,
    /// Background colour.
    pub bg: TermColor,
    /// Attributes.
    pub flags: CellFlags,
}

impl Default for SnapshotCell {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: TermColor::Foreground,
            bg: TermColor::Background,
            flags: CellFlags::empty(),
        }
    }
}

/// The changed part of one viewport row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineDamage {
    /// Viewport row.
    pub row: usize,
    /// First changed column.
    pub left: usize,
    /// Last changed column (inclusive).
    pub right: usize,
}

/// What changed since the previous snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Damage {
    /// Repaint everything (first frame, scroll, resize, mode change).
    #[default]
    Full,
    /// Only these rows changed (the cursor's old and new cells included).
    Lines(Vec<LineDamage>),
}

/// One frame's worth of terminal state: the visible cells, cursor, modes, scroll position,
/// selection, title and damage. Fill one with [`TermGrid::snapshot_into`] and keep it: its buffers
/// are reused frame to frame, so a steady stream allocates nothing.
#[derive(Debug, Clone, Default)]
pub struct TerminalSnapshot {
    /// Columns of the grid.
    pub columns: usize,
    /// Rows of the viewport.
    pub rows: usize,
    /// `rows x columns` cells, row-major.
    pub cells: Vec<SnapshotCell>,
    /// Combining characters, as `(index into cells, char)`, in cell order.
    pub zerowidth: Vec<(usize, char)>,
    /// The cursor.
    pub cursor: TerminalCursor,
    /// Modes in force.
    pub modes: TerminalModes,
    /// Lines the view is scrolled up into the history (`0` = live screen).
    pub display_offset: usize,
    /// Lines of history kept.
    pub history_size: usize,
    /// The selection as inclusive grid points (start before end), and whether it is a block.
    pub selection: Option<(GridPoint, GridPoint, bool)>,
    /// The title the process set.
    pub title: Option<Arc<str>>,
    /// Palette entries the process redefined (OSC 4 / 10 / 11), by [`TermColor`] palette index
    /// (`256` foreground, `257` background, `258` cursor).
    pub color_overrides: Vec<(usize, TermRgb)>,
    /// What changed since the previous snapshot.
    pub damage: Damage,
}

impl TerminalSnapshot {
    /// The cell at viewport `row`, `column`.
    pub fn cell(&self, row: usize, column: usize) -> Option<&SnapshotCell> {
        if column >= self.columns {
            return None;
        }
        self.cells.get(row * self.columns + column)
    }

    /// The text of viewport `row`, trailing blanks trimmed, wide-glyph spacers skipped
    /// (diagnostics and tests).
    pub fn row_text(&self, row: usize) -> String {
        let start = row * self.columns;
        let Some(cells) = self.cells.get(start..start + self.columns) else {
            return String::new();
        };
        let spacers = CellFlags::WIDE_CHAR_SPACER | CellFlags::LEADING_WIDE_CHAR_SPACER;
        let text: String = cells
            .iter()
            .filter(|cell| !cell.flags.intersects(spacers))
            .map(|cell| cell.c)
            .collect();
        text.trim_end().to_owned()
    }

    /// Whether the cell at viewport `row`, `column` is selected.
    pub fn is_selected(&self, row: usize, column: usize) -> bool {
        let Some((start, end, block)) = self.selection else {
            return false;
        };
        let point = GridPoint::from_viewport(row, column, self.display_offset);
        if block {
            return (start.line..=end.line).contains(&point.line)
                && (start.column.min(end.column)..=start.column.max(end.column))
                    .contains(&point.column);
        }
        start <= point && point <= end
    }
}

impl TermGrid {
    /// Copies the visible state into `out`, reusing its buffers, and resets the damage.
    pub fn snapshot_into(&mut self, out: &mut TerminalSnapshot) {
        match self.term.damage() {
            TermDamage::Full => out.damage = Damage::Full,
            TermDamage::Partial(lines) => {
                let mut reused = match std::mem::take(&mut out.damage) {
                    Damage::Lines(reused) => reused,
                    Damage::Full => Vec::new(),
                };
                reused.clear();
                reused.extend(lines.map(|line| LineDamage {
                    row: line.line,
                    left: line.left,
                    right: line.right,
                }));
                out.damage = Damage::Lines(reused);
            }
        }
        self.term.reset_damage();

        let columns = self.term.columns();
        let rows = self.term.screen_lines();
        let content = self.term.renderable_content();
        out.columns = columns;
        out.rows = rows;
        out.display_offset = content.display_offset;
        out.history_size = self.term.grid().history_size();
        out.modes = convert::modes(content.mode);
        out.title = self.title.clone();
        out.cursor = convert::cursor(content.cursor, content.display_offset, rows);
        out.selection = content
            .selection
            .map(|range| (range.start.into(), range.end.into(), range.is_block));
        out.color_overrides.clear();
        out.color_overrides.extend(
            (0..COLOR_COUNT)
                .filter_map(|index| content.colors[index].map(|rgb| (index, convert::rgb(rgb)))),
        );

        out.cells.clear();
        out.cells.resize(rows * columns, SnapshotCell::default());
        out.zerowidth.clear();
        let offset = content.display_offset as i32;
        for indexed in content.display_iter {
            let row = (indexed.point.line.0 + offset) as usize;
            let index = row * columns + indexed.point.column.0;
            let cell = indexed.cell;
            out.cells[index] = SnapshotCell {
                c: cell.c,
                fg: convert::color(cell.fg),
                bg: convert::color(cell.bg),
                flags: convert::flags(cell.flags),
            };
            if let Some(marks) = cell.zerowidth() {
                out.zerowidth.extend(marks.iter().map(|&c| (index, c)));
            }
        }
    }

    /// A fresh snapshot (allocates; frame loops use [`snapshot_into`](Self::snapshot_into)).
    pub fn snapshot(&mut self) -> TerminalSnapshot {
        let mut snapshot = TerminalSnapshot::default();
        self.snapshot_into(&mut snapshot);
        snapshot
    }
}
