//! Unit tests: feed VT sequences, assert the grid (E09-S04 AC 3).

mod scrollback;
mod search;
mod selection;
mod vt;
mod vt_screen;

use oxikube_ports::TerminalSize;

use super::{GridEvent, TermGrid, TerminalSnapshot};

/// A `columns x rows` grid with `scrollback` lines of history.
fn grid(columns: u16, rows: u16, scrollback: usize) -> TermGrid {
    TermGrid::new(TerminalSize::new(columns, rows), scrollback)
}

/// Feeds `bytes` and returns the events they produced.
fn feed(grid: &mut TermGrid, bytes: impl AsRef<[u8]>) -> Vec<GridEvent> {
    let mut events = Vec::new();
    grid.advance(bytes.as_ref(), &mut events);
    events
}

/// The text of every viewport row, trailing blanks trimmed.
fn screen(snapshot: &TerminalSnapshot) -> Vec<String> {
    (0..snapshot.rows)
        .map(|row| snapshot.row_text(row))
        .collect()
}
