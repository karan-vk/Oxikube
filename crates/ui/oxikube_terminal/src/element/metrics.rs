//! The terminal font and the cell geometry it gives: [`TerminalFont`] (family, size, line height)
//! and [`CellMetrics`] (cell width, line height, baseline), plus the conversions between pixels
//! and cells the element and its mouse handling use.

use gpui::{
    App, Bounds, Font, FontStyle, FontWeight, Pixels, Point, SharedString, Size, WindowTextSystem,
    font, point, px,
};
use oxikube_ports::TerminalSize;
use oxikube_settings::Settings as _;
use oxikube_ui::ActiveTokens as _;

use crate::grid::SelectionSide;
use crate::settings::TerminalSettings;

/// Line height as a multiple of the font size when `terminal.line_height` says nothing.
pub const DEFAULT_LINE_HEIGHT: f32 = 1.3;

/// The font a terminal draws with: the `terminal` font settings, falling back to the platform's
/// monospace family and the theme's monospace size.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalFont {
    /// Font family, e.g. `Menlo`.
    pub family: SharedString,
    /// Font size, zoom applied.
    pub size: Pixels,
    /// Line height as a multiple of [`size`](Self::size).
    pub line_height: f32,
}

impl TerminalFont {
    /// The platform's default monospace family: `Menlo` on macOS, `Consolas` on Windows,
    /// `DejaVu Sans Mono` elsewhere (what the component library's monospace text uses).
    pub fn platform_family() -> &'static str {
        if cfg!(target_os = "macos") {
            "Menlo"
        } else if cfg!(target_os = "windows") {
            "Consolas"
        } else {
            "DejaVu Sans Mono"
        }
    }

    /// The platform family at the theme's monospace size, with the UI zoom applied.
    pub fn from_theme(cx: &App) -> Self {
        Self {
            family: Self::platform_family().into(),
            size: oxikube_ui::u(cx.tokens().font.mono),
            line_height: DEFAULT_LINE_HEIGHT,
        }
    }

    /// The font the `terminal.font_family`, `font_size` and `line_height` settings ask for, each
    /// falling back to [`from_theme`](Self::from_theme) when unset (and all of it without a
    /// settings store). The UI zoom applies to an explicit size too. Cheap: it is read on every
    /// frame, and a change invalidates the shaped rows once.
    pub fn from_settings(cx: &App) -> Self {
        let mut font = Self::from_theme(cx);
        if let Some(settings) = TerminalSettings::try_get(cx) {
            if let Some(family) = &settings.font_family {
                font.family = family.clone();
            }
            if let Some(points) = settings.font_size {
                font.size = oxikube_ui::u(px(points));
            }
            font.line_height = settings.line_height;
        }
        font
    }

    /// The GPUI font of a cell: regular, bold and/or italic.
    pub fn font(&self, bold: bool, italic: bool) -> Font {
        let mut font = font(self.family.clone());
        if bold {
            font.weight = FontWeight::BOLD;
        }
        if italic {
            font.style = FontStyle::Italic;
        }
        font
    }
}

/// The size of one cell and where the text sits in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMetrics {
    /// Font size the runs are shaped at.
    pub font_size: Pixels,
    /// Advance of one cell (the font's `m`).
    pub cell_width: Pixels,
    /// Height of one row, whole pixels so stacked row backgrounds leave no seams.
    pub line_height: Pixels,
    /// Distance from the top of a row to the baseline.
    pub baseline: Pixels,
    /// Thickness of underlines and strikethroughs.
    pub stroke: Pixels,
}

impl CellMetrics {
    /// Metrics of `font` from the window's text system.
    pub fn measure(font: &TerminalFont, text_system: &WindowTextSystem) -> Self {
        let font_id = text_system.resolve_font(&font.font(false, false));
        let size = font.size;
        let cell_width = text_system
            .advance(font_id, size, 'm')
            .map(|advance| advance.width)
            .ok()
            .filter(|width| *width > px(0.))
            .unwrap_or(size * 0.6);
        let ascent = text_system.ascent(font_id, size);
        let descent = text_system.descent(font_id, size).abs();
        Self::from_parts(size, cell_width, font.line_height, ascent, descent)
    }

    /// Metrics from measured parts: the row is `size * line_height` rounded to whole pixels and
    /// the glyph box (`ascent + descent`) is centred in it.
    pub fn from_parts(
        size: Pixels,
        cell_width: Pixels,
        line_height: f32,
        ascent: Pixels,
        descent: Pixels,
    ) -> Self {
        let line_height = (size * line_height.max(1.0))
            .round()
            .max(ascent + descent)
            .max(px(1.));
        let baseline = (line_height - ascent - descent) / 2. + ascent;
        Self {
            font_size: size,
            cell_width,
            line_height,
            baseline,
            stroke: (size / 14.).round().max(px(1.)),
        }
    }

    /// The grid that fits in `size`: whole cells only, at least 2 x 1 (the grid's minimum), with
    /// the pixel size of those cells attached for the process (`TIOCGWINSZ`).
    pub fn grid_size(&self, size: Size<Pixels>) -> TerminalSize {
        let fit = |space: Pixels, cell: Pixels, min: u16| -> u16 {
            // The epsilon keeps an exact fit (480 / 4.8) from losing a cell to rounding.
            let cells = (space / cell + 1e-3).floor();
            if cells.is_finite() && cells >= f32::from(min) {
                cells.min(f32::from(u16::MAX)) as u16
            } else {
                min
            }
        };
        let columns = fit(size.width, self.cell_width, 2);
        let rows = fit(size.height, self.line_height, 1);
        let pixels = |cells: u16, cell: Pixels| (f32::from(cells) * cell.as_f32()).round() as u16;
        TerminalSize::new(columns, rows).with_pixels(
            pixels(columns, self.cell_width),
            pixels(rows, self.line_height),
        )
    }

    /// The top-left corner of viewport cell `row`, `column` in a grid drawn at `origin`.
    pub fn cell_origin(&self, origin: Point<Pixels>, row: usize, column: usize) -> Point<Pixels> {
        point(
            origin.x + self.cell_width * column as f32,
            origin.y + self.line_height * row as f32,
        )
    }

    /// The bounds of `cells` cells from viewport `row`, `column`.
    pub fn span(
        &self,
        origin: Point<Pixels>,
        row: usize,
        column: usize,
        cells: usize,
    ) -> Bounds<Pixels> {
        Bounds::new(
            self.cell_origin(origin, row, column),
            gpui::size(self.cell_width * cells as f32, self.line_height),
        )
    }

    /// The viewport cell under `position` in a `columns` x `rows` grid drawn at `origin`
    /// (clamped into the grid), and which half of it the pointer is on.
    pub fn cell_at(
        &self,
        origin: Point<Pixels>,
        position: Point<Pixels>,
        columns: usize,
        rows: usize,
    ) -> (usize, usize, SelectionSide) {
        let x = ((position.x - origin.x) / self.cell_width).max(0.);
        let y = ((position.y - origin.y) / self.line_height).max(0.);
        let column = (x.floor() as usize).min(columns.saturating_sub(1));
        let row = (y.floor() as usize).min(rows.saturating_sub(1));
        let side = if x.fract() < 0.5 {
            SelectionSide::Left
        } else {
            SelectionSide::Right
        };
        (row, column, side)
    }
}
