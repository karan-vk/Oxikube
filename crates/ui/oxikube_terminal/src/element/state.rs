//! [`TerminalElementState`]: what the element keeps between frames.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::Pixels;
use oxikube_ports::TerminalSize;

use super::cache::{CacheStats, RowCache};
use super::links::TerminalLink;
use super::metrics::{CellMetrics, TerminalFont};
use super::{blink, highlight, prepare};
use crate::grid::{SelectionSide, TerminalSnapshot};

/// What the element keeps between frames: the snapshot buffers, the shaped rows, the palette and
/// metrics it last used, and the hover and drag state. The host view creates one per terminal
/// and passes it to every [`TerminalElement`] it renders. Cheap to clone (a shared handle).
#[derive(Clone, Default)]
pub struct TerminalElementState(pub(super) Rc<RefCell<Inner>>);

#[derive(Default)]
pub(super) struct Inner {
    pub(super) snapshot: TerminalSnapshot,
    pub(super) cache: RowCache,
    /// The size the element last asked the terminal for.
    pub(super) requested: Option<TerminalSize>,
    pub(super) metrics: Option<(TerminalFont, CellMetrics)>,
    pub(super) palette: Option<prepare::PaletteMemo>,
    pub(super) hovered: Option<TerminalLink>,
    /// The content hash of each row the hovered link is on, when it was found.
    pub(super) hover_rows: Vec<(usize, u64)>,
    /// The cell the pointer was last over (link detection runs when it changes).
    pub(super) hover_cell: Option<(usize, usize)>,
    /// The cell a selection drag last reached; `None` when no drag is in progress.
    pub(super) dragging: Option<(usize, usize, SelectionSide)>,
    /// Wheel movement not yet worth a whole line.
    pub(super) scroll_remainder: Pixels,
    /// The button whose press was reported to the process (mouse reporting, E09-S06).
    pub(super) reported: Option<crate::mappings::mouse::MouseButton>,
    /// The cell the last mouse report named (a report is sent once per cell).
    pub(super) report_cell: Option<(usize, usize)>,
    /// The cursor's blink phase (E09-S11).
    pub(super) blink: blink::Blink,
    /// Search matches inside the viewport, rebuilt every frame.
    pub(super) highlight_spans: Vec<highlight::HighlightSpan>,
}

impl std::fmt::Debug for TerminalElementState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the snapshot: it holds what is on screen.
        f.debug_struct("TerminalElementState")
            .field("cache", &self.cache_stats())
            .finish_non_exhaustive()
    }
}

impl TerminalElementState {
    /// Fresh state: the first frame lays out and shapes every row.
    pub fn new() -> Self {
        Self::default()
    }

    /// The row cache's counters (tests and `--perf`).
    pub fn cache_stats(&self) -> CacheStats {
        self.0.borrow().cache.stats()
    }

    /// The link under the pointer while the platform modifier is held.
    pub fn hovered_link(&self) -> Option<TerminalLink> {
        self.0.borrow().hovered.clone()
    }
}
