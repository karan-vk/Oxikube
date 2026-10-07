//! The input method's composition, painted inline at the cursor (E09-S06): the marked text on the
//! terminal's background, underlined, starting at the cursor cell. Shaped each frame while a
//! composition exists (a handful of glyphs); nothing is cached because nothing is composing most
//! of the time.

use gpui::{App, SharedString, TextAlign, TextRun, UnderlineStyle, Window, fill};

use super::metrics::{CellMetrics, TerminalFont};
use super::palette::TerminalPalette;
use crate::grid::TerminalCursor;
use crate::input::ime::cells_in;

/// Paints `text` where the cursor is.
pub(super) fn paint(
    text: &str,
    cursor: TerminalCursor,
    origin: gpui::Point<gpui::Pixels>,
    font: &TerminalFont,
    metrics: CellMetrics,
    palette: &TerminalPalette,
    window: &mut Window,
    cx: &mut App,
) {
    let cells = cells_in(text).max(1);
    let area = metrics.span(origin, cursor.row, cursor.column, cells);
    window.paint_quad(fill(area, palette.background()));
    let color = palette.foreground();
    let run = TextRun {
        len: text.len(),
        font: font.font(false, false),
        color,
        background_color: None,
        underline: Some(UnderlineStyle {
            thickness: metrics.stroke,
            color: Some(color),
            wavy: false,
        }),
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(text.to_owned()),
        metrics.font_size,
        &[run],
        None,
    );
    // A failed glyph raster only loses that glyph.
    let _ = shaped.paint(
        area.origin,
        metrics.line_height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}
