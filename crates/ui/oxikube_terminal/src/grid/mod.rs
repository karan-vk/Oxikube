//! The terminal grid: `alacritty_terminal`'s `Term` and VT parser behind our own types (E09-S04).
//!
//! `alacritty_terminal` is pinned exactly (`=0.26.0`) and this module is the only code that names
//! its types (epic E09 risk: API churn). Everything it exposes is ours: [`TermGrid`] owns the
//! emulator, [`TerminalSnapshot`] is what the element paints, [`GridPoint`] / [`GridMatch`] /
//! [`SelectionKind`] address the grid, [`GridEvent`] is what the emulator asks of the outside world
//! (replies to the process, title, bell, clipboard).
//!
//! [`TermGrid`] is plain synchronous state: no GPUI, no async, no locking. The bridge
//! ([`crate::TerminalState`]) keeps it behind a short-lived mutex, feeds it on tokio and takes
//! snapshots on the UI thread.
//!
//! | Piece | Where |
//! |---|---|
//! | [`TermGrid`]: parse bytes, resize, scroll, scrollback limit | this file |
//! | [`TerminalSnapshot`] and its cell, colour, cursor, mode and damage types | `snapshot` |
//! | selection ([`SelectionKind`], [`GridPoint`], `selection_text`) | `selection` |
//! | search ([`GridMatch`], `search`) | `search` |
//! | [`GridEvent`], [`ColorRequest`] and the `EventListener` that collects them | `events` |
//! | alacritty → Oxikube type conversions | `convert` |
//!
//! Scrollback lives in memory only and is never persisted (non-negotiable 5).

mod convert;
mod events;
mod search;
mod selection;
mod snapshot;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Instant;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::{Config, Osc52, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use oxikube_ports::TerminalSize;

pub use events::{ColorRequest, GridEvent};
pub use search::{GridMatch, GridSearch, MAX_SEARCH_MATCHES, SEARCH_SLICE_LINES};
pub use selection::{GridPoint, SelectionKind, SelectionSide};
pub use snapshot::{
    CellFlags, CursorShape, Damage, LineDamage, SnapshotCell, TermColor, TermRgb, TerminalCursor,
    TerminalModes, TerminalSnapshot,
};

use events::GridListener;
use selection::SelectionRange;

/// Scrollback kept when the setting says nothing: 10 000 lines.
pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

/// The most scrollback a terminal keeps, whatever the setting says (alacritty's own cap). Memory
/// is bounded by `columns x (rows + scrollback)` cells.
pub const MAX_SCROLLBACK_LINES: usize = 100_000;

/// The smallest grid: two columns (a wide glyph must fit) by one row.
const MIN_COLUMNS: u16 = 2;
const MIN_ROWS: u16 = 1;

/// How far the visible region moves for [`TermGrid::scroll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalScroll {
    /// Move by this many lines: positive scrolls up into the history, negative back down.
    Lines(i32),
    /// One screen up.
    PageUp,
    /// One screen down.
    PageDown,
    /// The oldest line of the history.
    Top,
    /// The live screen (no scrollback showing).
    Bottom,
}

/// A grid size in cells, as alacritty's `Dimensions` wants it.
struct Cells {
    columns: usize,
    rows: usize,
}

impl Cells {
    fn new(size: TerminalSize) -> Self {
        Self {
            columns: usize::from(size.width),
            rows: usize::from(size.height),
        }
    }
}

impl Dimensions for Cells {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// The emulator configuration: `scrollback` lines of history, alacritty's defaults otherwise.
fn config(scrollback: usize) -> Config {
    Config {
        scrolling_history: scrollback,
        // A process may copy to the clipboard (OSC 52, surfaced as `GridEvent::ClipboardStore` for
        // the view to honour), never read it.
        osc52: Osc52::OnlyCopy,
        ..Config::default()
    }
}

/// `size` with at least [`MIN_COLUMNS`] x [`MIN_ROWS`] cells.
fn clamp_size(size: TerminalSize) -> TerminalSize {
    TerminalSize {
        width: size.width.max(MIN_COLUMNS),
        height: size.height.max(MIN_ROWS),
        ..size
    }
}

/// One terminal's emulator state: the cell grid with its scrollback, modes, cursor, selection and
/// title, plus the VT parser that turns a process's output into grid changes.
///
/// See the module docs. Every method is cheap except [`advance`](Self::advance) (linear in the
/// bytes) and [`search`](Self::search) (linear in the scrollback; [`GridSearch`] runs the same
/// scan in bounded slices).
pub struct TermGrid {
    term: Term<GridListener>,
    parser: Processor<StdSyncHandler>,
    listener: GridListener,
    size: TerminalSize,
    scrollback: usize,
    title: Option<Arc<str>>,
    /// The selection the last snapshot showed: alacritty's damage leaves selection changes out,
    /// so [`snapshot_into`](Self::snapshot_into) compares against this and damages the rows.
    painted_selection: Option<SelectionRange>,
    /// Bumped whenever lines move without new output (a resize reflows, a smaller scrollback
    /// drops history): a sliced [`GridSearch`] seeing it change starts over.
    layout_generation: u64,
}

impl std::fmt::Debug for TermGrid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the content: it can hold secrets.
        f.debug_struct("TermGrid")
            .field("size", &self.size)
            .field("scrollback", &self.scrollback)
            .finish_non_exhaustive()
    }
}

impl TermGrid {
    /// An empty grid of `size` cells keeping up to `scrollback_lines` lines of history (capped at
    /// [`MAX_SCROLLBACK_LINES`]).
    pub fn new(size: TerminalSize, scrollback_lines: usize) -> Self {
        let size = clamp_size(size);
        let scrollback = scrollback_lines.min(MAX_SCROLLBACK_LINES);
        let listener = GridListener::default();
        Self {
            term: Term::new(config(scrollback), &Cells::new(size), listener.clone()),
            parser: Processor::new(),
            listener,
            size,
            scrollback,
            title: None,
            painted_selection: None,
            layout_generation: 0,
        }
    }

    /// Parses `bytes` (process output) into the grid and appends what the emulator asks of the
    /// outside world to `events` (replies for the process first among them, in order).
    pub fn advance(&mut self, bytes: &[u8], events: &mut Vec<GridEvent>) {
        self.parser.advance(&mut self.term, bytes);
        self.drain_events(events);
    }

    /// When a synchronized update (`CSI ? 2026 h`) the process started must be applied even if
    /// it never ends it; `None` when no update is pending. Call [`flush_sync`](Self::flush_sync)
    /// at that instant.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    /// Applies a pending synchronized update now (its deadline passed).
    pub fn flush_sync(&mut self, events: &mut Vec<GridEvent>) {
        if self.sync_deadline().is_some() {
            self.parser.stop_sync(&mut self.term);
            self.drain_events(events);
        }
    }

    /// The grid size (at least 2 x 1 cells), with the pixel size it was given.
    pub fn size(&self) -> TerminalSize {
        self.size
    }

    /// Resizes the grid to `size` (clamped to at least 2 x 1), reflowing wrapped lines when the
    /// width changes. Returns the size actually applied.
    pub fn resize(&mut self, size: TerminalSize) -> TerminalSize {
        let size = clamp_size(size);
        if (size.width, size.height) != (self.size.width, self.size.height) {
            self.term.resize(Cells::new(size));
            self.layout_generation += 1;
        }
        self.size = size;
        size
    }

    /// Keeps at most `lines` lines of history from now on (capped at [`MAX_SCROLLBACK_LINES`]);
    /// shrinking drops the oldest lines at once.
    pub fn set_scrollback(&mut self, lines: usize) {
        let lines = lines.min(MAX_SCROLLBACK_LINES);
        if lines == self.scrollback {
            return;
        }
        self.scrollback = lines;
        self.term.set_options(config(lines));
        self.layout_generation += 1;
        // `set_options` re-announces the title; the title did not change.
        self.listener.discard();
    }

    /// The scrollback limit in force.
    pub fn scrollback_limit(&self) -> usize {
        self.scrollback
    }

    /// Lines of history currently kept above the screen.
    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    /// How many lines the view is scrolled up into the history (`0`: the live screen).
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Scrolls the visible region.
    pub fn scroll(&mut self, scroll: TerminalScroll) {
        let scroll = match scroll {
            TerminalScroll::Lines(lines) => Scroll::Delta(lines),
            TerminalScroll::PageUp => Scroll::PageUp,
            TerminalScroll::PageDown => Scroll::PageDown,
            TerminalScroll::Top => Scroll::Top,
            TerminalScroll::Bottom => Scroll::Bottom,
        };
        self.term.scroll_display(scroll);
        self.listener.discard();
    }

    /// The title the process set (OSC 0 / 2), if any.
    pub fn title(&self) -> Option<&Arc<str>> {
        self.title.as_ref()
    }

    /// The terminal modes in force (cursor keys, bracketed paste, mouse reporting, ...).
    pub fn modes(&self) -> TerminalModes {
        convert::modes(*self.term.mode())
    }

    /// Moves the collected emulator events into `out`, answering the ones the grid can answer
    /// itself (text-area size, colours the process set).
    fn drain_events(&mut self, out: &mut Vec<GridEvent>) {
        for event in self.listener.take() {
            if let Some(event) = self.translate(event) {
                out.push(event);
            }
        }
    }
}
