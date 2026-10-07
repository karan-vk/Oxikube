//! A row becomes background spans, style runs and decoration spans.

use oxikube_ports::TerminalSize;
use oxikube_theme::{Appearance, ThemeTokens};

use super::super::layout::{DecorationKind, RowLayout, selected_columns};
use super::super::palette::TerminalPalette;
use crate::grid::{GridEvent, GridPoint, SelectionKind, SelectionSide, TermGrid, TerminalSnapshot};

fn snapshot(columns: u16, rows: u16, bytes: &str) -> TerminalSnapshot {
    let mut grid = TermGrid::new(TerminalSize::new(columns, rows), 0);
    let mut events: Vec<GridEvent> = Vec::new();
    grid.advance(bytes.as_bytes(), &mut events);
    grid.snapshot()
}

fn palette() -> TerminalPalette {
    TerminalPalette::new(&ThemeTokens::fallback(Appearance::Dark).terminal)
}

fn row(snapshot: &TerminalSnapshot, row: usize) -> RowLayout {
    let mut layout = RowLayout::default();
    layout.build(snapshot, row, &palette());
    layout
}

/// `(column, text)` of every run.
fn runs(layout: &RowLayout) -> Vec<(usize, &str)> {
    layout
        .runs
        .iter()
        .map(|run| (run.column, &layout.text[run.bytes.clone()]))
        .collect()
}

#[test]
fn cells_of_one_style_are_one_run_and_blanks_only_join_runs() {
    let snap = snapshot(20, 1, "ab\x1b[1mcd\x1b[0m ef  gh   ");
    let layout = row(&snap, 0);
    assert_eq!(runs(&layout), [(0, "ab"), (2, "cd"), (5, "ef  gh")]);
    assert!(layout.runs[1].bold && !layout.runs[0].bold);
    assert!(
        layout.backgrounds.is_empty(),
        "default backgrounds are not spans"
    );
}

#[test]
fn colours_split_runs_and_backgrounds_merge() {
    let snap = snapshot(20, 1, "\x1b[31;41mabc\x1b[32;42mde\x1b[0m f\x1b[3mg");
    let layout = row(&snap, 0);
    let t = &ThemeTokens::fallback(Appearance::Dark).terminal;
    assert_eq!(runs(&layout), [(0, "abc"), (3, "de"), (6, "f"), (7, "g")]);
    assert_eq!(layout.runs[0].color, t.ansi.red);
    assert!(layout.runs[3].italic);
    let spans: Vec<_> = layout
        .backgrounds
        .iter()
        .map(|span| (span.column, span.cells, span.color))
        .collect();
    assert_eq!(spans, [(0, 3, t.ansi.red), (3, 2, t.ansi.green)]);
}

#[test]
fn a_wide_glyph_is_a_run_of_its_own_and_its_spacer_is_skipped() {
    let snap = snapshot(20, 1, "a你好b😀c");
    let layout = row(&snap, 0);
    assert_eq!(
        runs(&layout),
        [
            (0, "a"),
            (1, "你"),
            (3, "好"),
            (5, "b"),
            (6, "😀"),
            (8, "c")
        ]
    );
    assert!(layout.runs[1].wide && layout.runs[1].cells == 2);
    assert!(!layout.runs[0].wide);
}

#[test]
fn combining_marks_follow_their_base() {
    let snap = snapshot(20, 1, "e\u{301}x");
    let layout = row(&snap, 0);
    assert_eq!(runs(&layout), [(0, "e\u{301}x")]);
}

#[test]
fn underlines_and_strikethrough_are_spans() {
    let snap = snapshot(
        30,
        1,
        "\x1b[4mab\x1b[0m\x1b[4:3mcd\x1b[0m\x1b[4:2mef\x1b[0m\x1b[4:4mg\x1b[4:5mh\x1b[0m\x1b[9mij\x1b[0m",
    );
    let layout = row(&snap, 0);
    let spans: Vec<_> = layout
        .decorations
        .iter()
        .map(|span| (span.column, span.cells, span.kind))
        .collect();
    assert_eq!(
        spans,
        [
            (0, 2, DecorationKind::Underline),
            (2, 2, DecorationKind::CurlyUnderline),
            (4, 2, DecorationKind::DoubleUnderline),
            (6, 1, DecorationKind::DottedUnderline),
            (7, 1, DecorationKind::DashedUnderline),
            (8, 2, DecorationKind::Strikethrough),
        ]
    );
    assert_eq!(
        runs(&layout),
        [(0, "abcdefghij")],
        "decorations do not split runs"
    );
}

#[test]
fn hidden_cells_draw_no_text() {
    let snap = snapshot(20, 1, "a\x1b[8msecret\x1b[0mb");
    let layout = row(&snap, 0);
    // The hidden cells are blanks inside the run: nothing of them is shaped.
    assert_eq!(runs(&layout), [(0, "a      b")]);
    assert!(!layout.text.contains("secret"));
}

#[test]
fn selection_columns_per_row() {
    let mut grid = TermGrid::new(TerminalSize::new(10, 3), 0);
    let mut events = Vec::new();
    grid.advance(b"0123456789abcdefghijKLMNOPQRST", &mut events);
    grid.start_selection(
        SelectionKind::Cell,
        GridPoint::new(0, 4),
        SelectionSide::Left,
    );
    grid.update_selection(GridPoint::new(2, 2), SelectionSide::Right);
    let snap = grid.snapshot();
    assert_eq!(selected_columns(&snap, 0), Some(4..10));
    assert_eq!(selected_columns(&snap, 1), Some(0..10));
    assert_eq!(selected_columns(&snap, 2), Some(0..3));

    grid.start_selection(
        SelectionKind::Block,
        GridPoint::new(0, 3),
        SelectionSide::Left,
    );
    grid.update_selection(GridPoint::new(1, 6), SelectionSide::Right);
    let snap = grid.snapshot();
    assert_eq!(selected_columns(&snap, 0), Some(3..7));
    assert_eq!(selected_columns(&snap, 1), Some(3..7));
    assert_eq!(selected_columns(&snap, 2), None);
}
