//! What the element and the keymap do with a [`TerminalState`]: snapshot, resize, scroll,
//! selection, search and input. Each takes the grid lock briefly and never across an `.await`.

use std::sync::Arc;

use bytes::Bytes;
use gpui::{AppContext as _, Context, Task};
use oxikube_domain::OxiResult;
use oxikube_ports::TerminalSize;
use parking_lot::MutexGuard;

use super::TerminalState;
use crate::grid::{
    ColorRequest, GridMatch, GridPoint, GridSearch, SelectionKind, SelectionSide, TermRgb,
    TerminalModes, TerminalScroll, TerminalSnapshot,
};

impl TerminalState {
    /// Copies the visible state into `out`, reusing its buffers (the element keeps one snapshot
    /// and refills it each frame). Resets the damage: the next snapshot lists what changed since
    /// this one.
    pub fn snapshot_into(&self, out: &mut TerminalSnapshot) {
        self.grid.lock().snapshot_into(out);
    }

    /// [`snapshot_into`](Self::snapshot_into) unless the grid is busy (a large parse slice or a
    /// [`search`](Self::search) on the background executor holds it): then `out` is left as it
    /// was and this returns `false`. The element paints the previous frame and asks for another
    /// instead of waiting on the UI thread.
    pub fn try_snapshot_into(&self, out: &mut TerminalSnapshot) -> bool {
        match self.grid.try_lock() {
            Some(mut grid) => {
                grid.snapshot_into(out);
                true
            }
            None => false,
        }
    }

    /// A fresh snapshot (allocates; frame loops use [`snapshot_into`](Self::snapshot_into)).
    pub fn snapshot(&self) -> TerminalSnapshot {
        let mut snapshot = TerminalSnapshot::default();
        self.snapshot_into(&mut snapshot);
        snapshot
    }

    /// The grid size.
    pub fn size(&self) -> TerminalSize {
        self.grid.lock().size()
    }

    /// The modes the process switched on (for key and mouse encoding).
    pub fn modes(&self) -> TerminalModes {
        self.grid.lock().modes()
    }

    /// The title the process set, if any.
    pub fn title(&self) -> Option<Arc<str>> {
        self.grid.lock().title().cloned()
    }

    /// Resizes the grid now (the next frame shows the reflowed content) and tells the backend.
    /// Rapid resizes (a window drag) coalesce: the backend gets the latest size once the previous
    /// resize went out. Call it from layout with the size the element computed; a size that did
    /// not change costs a comparison.
    pub fn resize(&mut self, size: TerminalSize, cx: &mut Context<Self>) {
        let applied = self.grid.lock().resize(size);
        let changed = self.resize.send_if_modified(|current| {
            let changed = *current != applied;
            *current = applied;
            changed
        });
        if changed {
            cx.notify();
        }
    }

    /// Sends `bytes` (encoded keystrokes, a paste) to the process, after anything sent before.
    pub fn input(&self, bytes: impl Into<Bytes>) {
        // Closed only once the entity is gone.
        let _ = self.input.unbounded_send(bytes.into());
    }

    /// Answers a [`TerminalEvent::ColorRequest`](super::TerminalEvent::ColorRequest) with the
    /// theme's `colour`.
    pub fn reply_color(&self, request: &ColorRequest, colour: TermRgb) {
        self.input(request.reply(colour));
    }

    /// Scrolls the view through the history.
    pub fn scroll(&mut self, scroll: TerminalScroll, cx: &mut Context<Self>) {
        self.grid.lock().scroll(scroll);
        cx.notify();
    }

    /// Scrolls so `point` is visible (a search match).
    pub fn scroll_to(&mut self, point: GridPoint, cx: &mut Context<Self>) {
        self.grid.lock().scroll_to(point);
        cx.notify();
    }

    /// Back to the live screen (typing does this).
    pub fn scroll_to_bottom(&mut self, cx: &mut Context<Self>) {
        self.scroll(TerminalScroll::Bottom, cx);
    }

    /// Starts a selection of `kind` at `point`.
    pub fn start_selection(
        &mut self,
        kind: SelectionKind,
        point: GridPoint,
        side: SelectionSide,
        cx: &mut Context<Self>,
    ) {
        self.grid.lock().start_selection(kind, point, side);
        cx.notify();
    }

    /// Extends the selection to `point`.
    pub fn update_selection(
        &mut self,
        point: GridPoint,
        side: SelectionSide,
        cx: &mut Context<Self>,
    ) {
        self.grid.lock().update_selection(point, side);
        cx.notify();
    }

    /// Drops the selection.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.grid.lock().clear_selection();
        cx.notify();
    }

    /// The selected text (for copy), `None` without a selection. Never logged.
    pub fn selection_text(&self) -> Option<String> {
        self.grid.lock().selection_text()
    }

    /// Every match of `pattern` over screen and scrollback (see [`crate::grid::GridSearch`]),
    /// computed on the background executor in slices of
    /// [`SEARCH_SLICE_LINES`](crate::grid::SEARCH_SLICE_LINES) lines (about 0.4 ms each in
    /// release builds). The grid lock is handed back between slices, so scrolling, selecting,
    /// resizing and painting go on during a long search; new output waits until it ends, so the
    /// matches are consistent. Matches are grid points as of the search: output that arrives
    /// afterwards shifts them.
    ///
    /// # Errors
    ///
    /// The task yields `Validation` when `pattern` is not a valid regex.
    pub fn search(&self, pattern: &str, cx: &mut Context<Self>) -> Task<OxiResult<Vec<GridMatch>>> {
        let grid = self.grid.clone();
        let gate = self.output_gate.clone();
        let pattern = pattern.to_owned();
        cx.background_spawn(async move {
            let mut search = GridSearch::new(&pattern)?;
            let _frozen = gate.lock().await;
            loop {
                let locked = grid.lock();
                let done = search.step(&locked);
                // Hand the lock straight to a waiter (the UI thread) rather than re-taking it.
                MutexGuard::unlock_fair(locked);
                if done {
                    return Ok(search.into_matches());
                }
            }
        })
    }
}
