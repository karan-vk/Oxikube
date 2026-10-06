//! Screen-level sequences: alternate screen, wide glyphs, scroll regions, combining marks,
//! synchronized updates and damage.

use super::super::{CellFlags, Damage, TerminalModes};
use super::{feed, grid, screen};

#[test]
fn alternate_screen_saves_and_restores_the_primary_one() {
    let mut grid = grid(10, 3, 100);
    feed(&mut grid, "$ vim\r\n");
    feed(&mut grid, "\x1b[?1049h\x1b[H\x1b[2Jeditor");
    let snap = grid.snapshot();
    assert!(snap.modes.contains(TerminalModes::ALT_SCREEN));
    assert_eq!(screen(&snap), ["editor", "", ""]);

    feed(&mut grid, "\x1b[?1049l");
    let snap = grid.snapshot();
    assert!(!snap.modes.contains(TerminalModes::ALT_SCREEN));
    assert_eq!(screen(&snap), ["$ vim", "", ""]);
    assert_eq!((snap.cursor.row, snap.cursor.column), (1, 0));
}

#[test]
fn alternate_screen_output_never_reaches_the_scrollback() {
    let mut grid = grid(10, 2, 100);
    feed(&mut grid, "\x1b[?1049h");
    for line in 0..20 {
        feed(&mut grid, format!("line {line}\r\n"));
    }
    assert_eq!(grid.history_size(), 0);
}

#[test]
fn wide_glyphs_take_two_cells() {
    let mut grid = grid(10, 2, 0);
    feed(&mut grid, "a中b");
    let snap = grid.snapshot();
    let wide = snap.cell(0, 1).unwrap();
    assert_eq!(wide.c, '中');
    assert!(wide.flags.contains(CellFlags::WIDE_CHAR));
    assert!(
        snap.cell(0, 2)
            .unwrap()
            .flags
            .contains(CellFlags::WIDE_CHAR_SPACER)
    );
    assert_eq!(snap.cell(0, 3).unwrap().c, 'b');
    assert_eq!(snap.row_text(0), "a中b");
    assert_eq!(snap.cursor.column, 4);
}

#[test]
fn a_wide_glyph_that_does_not_fit_wraps_with_a_leading_spacer() {
    let mut grid = grid(4, 2, 0);
    feed(&mut grid, "abc中");
    let snap = grid.snapshot();
    assert!(
        snap.cell(0, 3)
            .unwrap()
            .flags
            .contains(CellFlags::LEADING_WIDE_CHAR_SPACER)
    );
    assert_eq!(snap.cell(1, 0).unwrap().c, '中');
    assert_eq!(grid.selection_text(), None);
}

#[test]
fn combining_marks_ride_on_their_cell() {
    let mut grid = grid(10, 1, 0);
    feed(&mut grid, "e\u{301}x");
    let snap = grid.snapshot();
    assert_eq!(snap.cell(0, 0).unwrap().c, 'e');
    assert_eq!(snap.zerowidth, [(0, '\u{301}')]);
    assert_eq!(snap.cell(0, 1).unwrap().c, 'x');
}

#[test]
fn scroll_region_scrolls_only_its_lines() {
    let mut grid = grid(10, 5, 100);
    feed(&mut grid, "top\r\n1\r\n2\r\n3\r\nbottom");
    // Region rows 2..4 (1-based), cursor to its last row, two line feeds.
    feed(&mut grid, "\x1b[2;4r\x1b[4;1H\n\nnew");
    let snap = grid.snapshot();
    assert_eq!(screen(&snap), ["top", "3", "", "new", "bottom"]);
    assert_eq!(
        grid.history_size(),
        0,
        "a partial region scrolls nothing into history"
    );
}

#[test]
fn reverse_index_at_the_top_scrolls_down() {
    let mut grid = grid(10, 3, 0);
    feed(&mut grid, "a\r\nb\r\nc\x1b[H\x1bM");
    assert_eq!(screen(&grid.snapshot()), ["", "a", "b"]);
}

#[test]
fn synchronized_update_is_held_until_its_end() {
    let mut grid = grid(10, 2, 0);
    let mut events = Vec::new();
    grid.advance(b"\x1b[?2026hhalf", &mut events);
    assert_eq!(grid.snapshot().row_text(0), "", "nothing shows mid-update");
    assert!(grid.sync_deadline().is_some());
    grid.advance(b" done\x1b[?2026l", &mut events);
    assert_eq!(grid.snapshot().row_text(0), "half done");
    assert!(grid.sync_deadline().is_none());

    // An update that never ends is applied when its deadline is flushed.
    grid.advance(b"\r\x1b[?2026hstuck", &mut events);
    assert_eq!(grid.snapshot().row_text(0), "half done");
    grid.flush_sync(&mut events);
    assert_eq!(grid.snapshot().row_text(0), "stuckdone");
}

#[test]
fn damage_lists_changed_rows_then_resets() {
    let mut grid = grid(10, 4, 0);
    assert_eq!(
        grid.snapshot().damage,
        Damage::Full,
        "the first frame repaints everything"
    );
    feed(&mut grid, "\x1b[3;1Hxy");
    let Damage::Lines(lines) = grid.snapshot().damage else {
        panic!("expected partial damage");
    };
    let rows: Vec<usize> = lines.iter().map(|line| line.row).collect();
    assert!(rows.contains(&2), "the written row is damaged: {rows:?}");
    assert!(
        rows.contains(&0),
        "the cursor's old row is damaged: {rows:?}"
    );
    assert!(!rows.contains(&1), "untouched rows are not: {rows:?}");
}
