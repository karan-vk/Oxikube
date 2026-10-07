//! OSC 8 hyperlinks in the snapshot (E09-S05).

use std::sync::Arc;

use super::super::TerminalSnapshot;
use super::{feed, grid};

const LINK: &str = "\x1b]8;;https://example.com/a\x1b\\docs\x1b]8;;\x1b\\ plain \x1b]8;id=x;file:///tmp/b\x07b\x1b]8;;\x07";

#[test]
fn cells_carry_their_link() {
    let mut grid = grid(20, 2, 0);
    feed(&mut grid, LINK);
    let snap = grid.snapshot();
    assert_eq!(snap.row_text(0), "docs plain b");
    for column in 0..4 {
        assert_eq!(
            snap.hyperlink_at(0, column).map(|uri| &**uri),
            Some("https://example.com/a")
        );
    }
    assert_eq!(snap.hyperlink_at(0, 4), None, "the space after the link");
    assert_eq!(
        snap.hyperlink_at(0, 11).map(|uri| &**uri),
        Some("file:///tmp/b")
    );
    assert_eq!(snap.hyperlink_at(1, 0), None);
    assert_eq!(snap.hyperlink_at(0, 99), None, "outside the grid");
    assert_eq!(snap.hyperlinks.len(), 5);
}

#[test]
fn a_link_that_stays_on_screen_is_not_copied_again() {
    let mut grid = grid(20, 2, 0);
    feed(&mut grid, LINK);
    let mut snap = TerminalSnapshot::default();
    grid.snapshot_into(&mut snap);
    let first = snap.hyperlink_at(0, 0).cloned().unwrap();
    feed(&mut grid, "\r\n more output");
    grid.snapshot_into(&mut snap);
    let again = snap.hyperlink_at(0, 0).unwrap();
    assert!(Arc::ptr_eq(&first, again), "the interned URI is reused");
    assert_eq!(snap.hyperlink_uris.len(), 2);
}
