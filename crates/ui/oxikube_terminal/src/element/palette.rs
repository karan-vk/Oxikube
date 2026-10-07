//! [`TerminalPalette`]: a cell's colours resolved against the theme's terminal tokens.
//!
//! The 16 ANSI colours, the dim row, the default foreground and background, the cursor and the
//! selection come from `oxikube_theme` ([`TerminalColors`]); nothing here hard-codes a palette.
//! Entries `16..232` (the 6 x 6 x 6 colour cube) and `232..256` (the grey ramp) are the xterm
//! formulas, and 24-bit colours pass through. Colours the process redefined (OSC 4 / 10 / 11 /
//! 12) override the theme for that terminal.

use gpui::{Hsla, Rgba};
use oxikube_theme::tokens::{AnsiColors, TerminalColors};

use crate::grid::{CellFlags, SnapshotCell, TermColor, TermRgb};

/// Palette indices of the special colours alacritty reports overrides for (its `NamedColor`
/// numbering): foreground, background, cursor, the dim row, bright and dim foreground.
const FOREGROUND: usize = 256;
const BACKGROUND: usize = 257;
const CURSOR: usize = 258;
const DIM_BLACK: usize = 259;
const BRIGHT_FOREGROUND: usize = 267;
const DIM_FOREGROUND: usize = 268;

/// How much a faint (SGR 2) colour keeps of its opacity when the palette has no dim variant.
const DIM_ALPHA: f32 = 0.66;

/// Every colour a terminal can name, resolved. Build one per theme (and per set of process
/// overrides); resolving a cell is then a table lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalPalette {
    indexed: [Hsla; 256],
    dim: [Hsla; 8],
    foreground: Hsla,
    background: Hsla,
    bright_foreground: Hsla,
    dim_foreground: Hsla,
    cursor: Hsla,
    selection: Hsla,
}

impl TerminalPalette {
    /// The palette of a theme.
    pub fn new(theme: &TerminalColors) -> Self {
        let mut indexed = [theme.foreground; 256];
        indexed[..8].copy_from_slice(&ansi_row(&theme.ansi));
        indexed[8..16].copy_from_slice(&ansi_row(&theme.bright));
        for (slot, index) in indexed[16..].iter_mut().zip(16u8..=255) {
            *slot = rgb(xterm_rgb(index));
        }
        Self {
            indexed,
            dim: ansi_row(&theme.dim),
            foreground: theme.foreground,
            background: theme.background,
            bright_foreground: theme.bright_foreground,
            dim_foreground: theme.dim_foreground,
            cursor: theme.cursor,
            selection: theme.selection,
        }
    }

    /// The same palette with the colours a process redefined, by alacritty palette index (what
    /// [`TerminalSnapshot::color_overrides`](crate::grid::TerminalSnapshot::color_overrides)
    /// holds).
    #[must_use]
    pub fn with_overrides(mut self, overrides: &[(usize, TermRgb)]) -> Self {
        for &(index, color) in overrides {
            let color = rgb(color);
            match index {
                0..256 => self.indexed[index] = color,
                FOREGROUND => self.foreground = color,
                BACKGROUND => self.background = color,
                CURSOR => self.cursor = color,
                DIM_BLACK..BRIGHT_FOREGROUND => self.dim[index - DIM_BLACK] = color,
                BRIGHT_FOREGROUND => self.bright_foreground = color,
                DIM_FOREGROUND => self.dim_foreground = color,
                _ => {}
            }
        }
        self
    }

    /// The default background (what the element fills with).
    pub fn background(&self) -> Hsla {
        self.background
    }

    /// The default foreground.
    pub fn foreground(&self) -> Hsla {
        self.foreground
    }

    /// The cursor colour.
    pub fn cursor(&self) -> Hsla {
        self.cursor
    }

    /// The selection highlight.
    pub fn selection(&self) -> Hsla {
        self.selection
    }

    /// `color` resolved.
    pub fn resolve(&self, color: TermColor) -> Hsla {
        match color {
            TermColor::Foreground => self.foreground,
            TermColor::Background => self.background,
            TermColor::Cursor => self.cursor,
            TermColor::BrightForeground => self.bright_foreground,
            TermColor::DimForeground => self.dim_foreground,
            TermColor::Indexed(index) => self.indexed[usize::from(index)],
            TermColor::Dim(index) => self.dim[usize::from(index) & 7],
            TermColor::Rgb(color) => rgb(color),
        }
    }

    /// The foreground and background a cell is painted with, its attributes applied: bold draws
    /// the default foreground bright, faint draws dim, inverse swaps the two, hidden draws the
    /// text in the background colour. The background is `None` when it is the default one
    /// (the element has filled it already).
    pub fn cell_colors(&self, cell: &SnapshotCell) -> (Hsla, Option<Hsla>) {
        let flags = cell.flags;
        let mut fg = match cell.fg {
            TermColor::Foreground if flags.contains(CellFlags::DIM) => self.dim_foreground,
            TermColor::Foreground if flags.contains(CellFlags::BOLD) => self.bright_foreground,
            TermColor::Indexed(index @ 0..8) if flags.contains(CellFlags::DIM) => {
                self.dim[usize::from(index)]
            }
            other if flags.contains(CellFlags::DIM) => {
                let color = self.resolve(other);
                color.opacity(DIM_ALPHA)
            }
            other => self.resolve(other),
        };
        let mut bg = (cell.bg != TermColor::Background).then(|| self.resolve(cell.bg));
        if flags.contains(CellFlags::INVERSE) {
            let back = bg.unwrap_or(self.background);
            bg = Some(fg);
            fg = back;
        }
        if flags.contains(CellFlags::HIDDEN) {
            fg = bg.unwrap_or(self.background);
        }
        (fg, bg)
    }
}

fn ansi_row(row: &AnsiColors) -> [Hsla; 8] {
    [
        row.black,
        row.red,
        row.green,
        row.yellow,
        row.blue,
        row.magenta,
        row.cyan,
        row.white,
    ]
}

/// The xterm RGB value of palette entry `index` in `16..=255`: the 6 x 6 x 6 cube, then 24
/// greys from 8 to 238.
fn xterm_rgb(index: u8) -> TermRgb {
    if index >= 232 {
        let grey = 8 + (index - 232) * 10;
        return TermRgb {
            r: grey,
            g: grey,
            b: grey,
        };
    }
    let cube = index.saturating_sub(16);
    let level = |step: u8| if step == 0 { 0 } else { 55 + step * 40 };
    TermRgb {
        r: level(cube / 36),
        g: level((cube / 6) % 6),
        b: level(cube % 6),
    }
}

fn rgb(color: TermRgb) -> Hsla {
    Hsla::from(Rgba {
        r: f32::from(color.r) / 255.,
        g: f32::from(color.g) / 255.,
        b: f32::from(color.b) / 255.,
        a: 1.,
    })
}
