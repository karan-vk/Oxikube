//! The paint passes, in order: background, cell backgrounds, selection, text runs, cursor,
//! decorations (underlines, strikethrough), the hovered link's underline, the scroll thumb.
//!
//! Everything is drawn from the row cache and the snapshot prepaint left behind; nothing is laid
//! out or shaped here except the one glyph under a block cursor, which the row cache keeps (and
//! drops with its rows when the font, metrics or palette change).

use gpui::{
    App, BorderStyle, Bounds, Hsla, Pixels, Point, TextAlign, UnderlineStyle, Window, fill,
    outline, point, px, size,
};
use oxikube_theme::ActiveTheme;

use super::cache::RowCache;
use super::layout::{DecorationKind, DecorationSpan, selected_columns};
use super::metrics::CellMetrics;
use super::palette::TerminalPalette;
use super::{Inner, TerminalElementState, TerminalFrame};
use crate::grid::{CellFlags, CursorShape, TerminalSnapshot};

/// Width of a beam cursor and height of an underline cursor, in strokes.
const CURSOR_STROKES: f32 = 2.;
/// Width of the scroll thumb.
const THUMB_WIDTH: Pixels = px(4.);
/// Shortest scroll thumb.
const THUMB_MIN: Pixels = px(16.);

/// Paints the frame prepaint prepared.
pub(super) fn paint(
    state: &TerminalElementState,
    frame: &TerminalFrame,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let mut inner = state.0.borrow_mut();
    let Inner {
        snapshot,
        cache,
        palette,
        hovered,
        ..
    } = &mut *inner;
    let Some(palette) = palette.as_ref().map(|memo| &memo.palette) else {
        return;
    };
    let metrics = frame.metrics;
    let origin = frame.origin;
    window.paint_quad(fill(bounds, palette.background()));
    for (row, cached) in cache.rows().enumerate() {
        for span in &cached.layout.backgrounds {
            let area = metrics.span(origin, row, span.column, span.cells);
            window.paint_quad(fill(area, span.color));
        }
    }
    for row in 0..snapshot.rows {
        if let Some(columns) = selected_columns(snapshot, row) {
            let area = metrics.span(origin, row, columns.start, columns.len());
            window.paint_quad(fill(area, palette.selection()));
        }
    }
    for (row, cached) in cache.rows().enumerate() {
        for (run, shaped) in cached.layout.runs.iter().zip(&cached.shaped) {
            let at = metrics.cell_origin(origin, row, run.column);
            // A failed glyph raster only loses that glyph; the frame goes on.
            let _ = shaped.paint(at, metrics.line_height, TextAlign::Left, None, window, cx);
        }
    }
    paint_cursor(snapshot, palette, frame, cache, window, cx);
    for (row, cached) in cache.rows().enumerate() {
        for span in &cached.layout.decorations {
            paint_decoration(span, row, origin, metrics, window);
        }
    }
    if let Some(link) = hovered.as_ref() {
        for &(row, first, last) in &link.cells {
            let color = snapshot
                .cell(row, first)
                .map_or(palette.foreground(), |cell| palette.cell_colors(cell).0);
            let span = DecorationSpan {
                column: first,
                cells: last + 1 - first,
                kind: DecorationKind::Underline,
                color,
            };
            paint_decoration(&span, row, origin, metrics, window);
        }
    }
    paint_scroll_thumb(snapshot, bounds, window, cx);
}

/// The cursor: a block (with the glyph under it redrawn in the cell's background colour), a beam,
/// an underline, or a hollow block when the terminal is not focused.
fn paint_cursor(
    snapshot: &TerminalSnapshot,
    palette: &TerminalPalette,
    frame: &TerminalFrame,
    cache: &mut RowCache,
    window: &mut Window,
    cx: &mut App,
) {
    let cursor = snapshot.cursor;
    if !cursor.visible {
        return;
    }
    let Some(cell) = snapshot.cell(cursor.row, cursor.column) else {
        return;
    };
    let metrics = frame.metrics;
    let cells = if cell.flags.contains(CellFlags::WIDE_CHAR) {
        2
    } else {
        1
    };
    let area = metrics.span(frame.origin, cursor.row, cursor.column, cells);
    let color = palette.cursor();
    let stroke = metrics.stroke * CURSOR_STROKES;
    let shape = if frame.focused {
        cursor.shape
    } else {
        CursorShape::HollowBlock
    };
    match shape {
        CursorShape::Block => {
            window.paint_quad(fill(area, color));
            if matches!(cell.c, ' ' | '\0') || cell.flags.contains(CellFlags::HIDDEN) {
                return;
            }
            if let Some(shaped) = cache.cursor_glyph(cell, window.text_system()) {
                let _ = shaped.paint(
                    area.origin,
                    metrics.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        }
        CursorShape::Beam => {
            window.paint_quad(fill(
                Bounds::new(area.origin, size(stroke, area.size.height)),
                color,
            ));
        }
        CursorShape::Underline => {
            let top = area.bottom() - stroke;
            window.paint_quad(fill(
                Bounds::new(point(area.origin.x, top), size(area.size.width, stroke)),
                color,
            ));
        }
        CursorShape::HollowBlock => {
            window.paint_quad(outline(area, color, BorderStyle::Solid));
        }
    }
}

/// One decoration span of viewport `row`.
fn paint_decoration(
    span: &DecorationSpan,
    row: usize,
    origin: Point<Pixels>,
    metrics: CellMetrics,
    window: &mut Window,
) {
    let area = metrics.span(origin, row, span.column, span.cells);
    let stroke = metrics.stroke;
    // Just below the baseline, kept inside the row.
    let under = (area.origin.y + metrics.baseline + stroke).min(area.bottom() - stroke * 2.);
    let line = |y: Pixels, x: Pixels, width: Pixels| {
        fill(Bounds::new(point(x, y), size(width, stroke)), span.color)
    };
    match span.kind {
        DecorationKind::Underline => window.paint_quad(line(under, area.origin.x, area.size.width)),
        DecorationKind::DoubleUnderline => {
            window.paint_quad(line(under - stroke, area.origin.x, area.size.width));
            window.paint_quad(line(under + stroke, area.origin.x, area.size.width));
        }
        DecorationKind::CurlyUnderline => window.paint_underline(
            point(area.origin.x, under - stroke),
            area.size.width,
            &UnderlineStyle {
                thickness: stroke,
                color: Some(span.color),
                wavy: true,
            },
        ),
        DecorationKind::DottedUnderline | DecorationKind::DashedUnderline => {
            let (dash, gap) = if span.kind == DecorationKind::DottedUnderline {
                (stroke, stroke)
            } else {
                (stroke * 3., stroke * 2.)
            };
            let mut x = area.origin.x;
            while x < area.right() {
                let width = dash.min(area.right() - x);
                window.paint_quad(line(under, x, width));
                x += dash + gap;
            }
        }
        DecorationKind::Strikethrough => {
            let middle = area.origin.y + metrics.baseline - metrics.font_size * 0.3;
            window.paint_quad(line(middle, area.origin.x, area.size.width));
        }
    }
}

/// A thin thumb on the right edge while the view is scrolled into the history.
fn paint_scroll_thumb(
    snapshot: &TerminalSnapshot,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    if snapshot.display_offset == 0 || snapshot.history_size == 0 {
        return;
    }
    let history = snapshot.history_size as f32;
    let total = history + snapshot.rows as f32;
    let height = (bounds.size.height * (snapshot.rows as f32 / total)).max(THUMB_MIN);
    // 0 at the oldest line of the history, 1 at the live screen.
    let fraction = (history - snapshot.display_offset.min(snapshot.history_size) as f32) / history;
    let top = bounds.origin.y + (bounds.size.height - height).max(px(0.)) * fraction;
    let color: Hsla = ActiveTheme::get(cx).colors.scrollbar_thumb;
    window.paint_quad(fill(
        Bounds::new(
            point(bounds.right() - THUMB_WIDTH, top),
            size(THUMB_WIDTH, height),
        ),
        color,
    ));
}
