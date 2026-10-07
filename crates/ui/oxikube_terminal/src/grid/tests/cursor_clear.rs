//! The default cursor (`terminal.cursor_shape` / `cursor_blink`), select all and clear (E09-S11).

use super::super::{CursorShape, DefaultCursor, TerminalScroll};
use super::{feed, grid, screen};

const BAR: DefaultCursor = DefaultCursor {
    shape: CursorShape::Beam,
    blinking: true,
};

#[test]
fn the_cursor_is_a_steady_block_until_told_otherwise() {
    let mut grid = grid(20, 3, 0);
    let cursor = grid.snapshot().cursor;
    assert_eq!((cursor.shape, cursor.blinking), (CursorShape::Block, false));
    assert_eq!(grid.default_cursor(), DefaultCursor::default());
}

#[test]
fn the_default_cursor_shows_until_the_process_sets_its_own() {
    let mut grid = grid(20, 3, 0);
    grid.set_default_cursor(BAR);
    let cursor = grid.snapshot().cursor;
    assert_eq!((cursor.shape, cursor.blinking), (CursorShape::Beam, true));

    // DECSCUSR 4: a steady underline, until it is reset.
    feed(&mut grid, "\x1b[4 q");
    let cursor = grid.snapshot().cursor;
    assert_eq!(
        (cursor.shape, cursor.blinking),
        (CursorShape::Underline, false)
    );
    // The setting changing meanwhile does not override the process...
    grid.set_default_cursor(DefaultCursor {
        shape: CursorShape::Block,
        blinking: false,
    });
    assert_eq!(grid.snapshot().cursor.shape, CursorShape::Underline);
    // ...which is back to the default when it asks for it (DECSCUSR 0).
    feed(&mut grid, "\x1b[0 q");
    assert_eq!(grid.snapshot().cursor.shape, CursorShape::Block);
}

#[test]
fn a_cursor_change_does_not_touch_the_title_or_the_scrollback() {
    let mut grid = grid(20, 3, 50);
    feed(&mut grid, "\x1b]0;keep me\x07");
    feed(&mut grid, "a\r\nb\r\nc\r\nd\r\n");
    let history = grid.history_size();
    grid.set_default_cursor(BAR);
    assert_eq!(grid.history_size(), history);
    assert_eq!(grid.title().map(|title| &**title), Some("keep me"));
    let mut events = Vec::new();
    grid.advance(b"", &mut events);
    assert!(events.is_empty(), "no phantom title event");
}

#[test]
fn select_all_takes_the_scrollback_and_the_screen() {
    let mut grid = grid(10, 2, 10);
    feed(&mut grid, "one\r\ntwo\r\nthree\r\nfour");
    grid.select_all();
    assert_eq!(
        grid.selection_text().as_deref(),
        Some("one\ntwo\nthree\nfour")
    );
    // Scrolled into the history it still selects everything.
    grid.scroll(TerminalScroll::Top);
    grid.select_all();
    assert_eq!(
        grid.selection_text().as_deref(),
        Some("one\ntwo\nthree\nfour")
    );
}

#[test]
fn clear_drops_the_history_and_moves_the_cursor_line_to_the_top() {
    let mut grid = grid(20, 4, 100);
    feed(&mut grid, "a\r\nb\r\nc\r\nd\r\ne\r\n$ typed");
    assert!(grid.history_size() > 0);
    grid.scroll(TerminalScroll::Top);
    grid.select_all();

    grid.clear();
    assert_eq!(grid.history_size(), 0, "the scrollback is gone");
    assert_eq!(grid.display_offset(), 0, "back on the live screen");
    assert_eq!(grid.selection_text(), None, "the selection went with it");
    let snapshot = grid.snapshot();
    assert_eq!(screen(&snapshot), ["$ typed", "", "", ""]);
    assert_eq!((snapshot.cursor.row, snapshot.cursor.column), (0, 7));

    // The process goes on where it was.
    feed(&mut grid, "!");
    assert_eq!(grid.snapshot().row_text(0), "$ typed!");
}

#[test]
fn clear_on_the_alternate_screen_changes_nothing() {
    let mut grid = grid(20, 3, 100);
    feed(&mut grid, "one\r\ntwo\r\nthree\r\nfour\r\n");
    let history = grid.history_size();
    feed(&mut grid, "\x1b[?1049h\x1b[Hvim");
    grid.clear();
    assert_eq!(grid.snapshot().row_text(0), "vim");
    feed(&mut grid, "\x1b[?1049l");
    assert_eq!(
        grid.history_size(),
        history,
        "the main screen kept its history"
    );
}

#[test]
fn a_cleared_grid_forgets_what_it_held_for_search() {
    let mut grid = grid(20, 3, 100);
    feed(&mut grid, "needle\r\nhay\r\nhay\r\nhay\r\n");
    assert_eq!(grid.search("needle").unwrap().len(), 1);
    grid.clear();
    assert!(grid.search("needle").unwrap().is_empty());
    assert_eq!(grid.snapshot().cursor.row, 0);
}
