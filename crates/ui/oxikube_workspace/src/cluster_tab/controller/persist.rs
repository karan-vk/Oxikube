//! Saving which tabs are open, in what order, and which is displayed.
//!
//! Changes mark the state dirty and (re)start a debounce timer; when it fires the snapshot is
//! captured on the UI thread (a walk over the open tabs) and written off it. An unchanged
//! snapshot is not written again, the controller writes nothing until something happens (so a
//! window that starts empty never overwrites what session restore, E06-S11, is about to read),
//! and a quit writes what is pending and waits for it.

use std::future::Future;

use gpui::{Context, Task};

use super::ClusterTabs;
use crate::cluster_tab::store::SavedTabs;
use crate::persistence::SAVE_DEBOUNCE;

/// The debounce and write bookkeeping of [`ClusterTabs`].
#[derive(Default)]
pub(super) struct DebouncedSave {
    dirty: bool,
    last_written: Option<SavedTabs>,
    debounce_task: Option<Task<()>>,
    write_task: Option<Task<()>>,
}

impl ClusterTabs {
    /// The layout of the window's workspace changed: a tab may have been dragged.
    pub(super) fn layout_changed(&mut self, cx: &mut Context<Self>) {
        // Dragging only changes the order; skip the timer when the order is what was last
        // written or pending.
        let order = self.display_order(cx);
        let unchanged = self.save.last_written.as_ref().map(|saved| &saved.open) == Some(&order)
            && !self.save.dirty;
        if !unchanged && !self.tabs.is_empty() {
            self.mark_dirty(cx);
        }
    }

    /// Something saved changed: write soon, once changes stop for [`SAVE_DEBOUNCE`].
    pub(super) fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        self.save.dirty = true;
        // Replacing the timer cancels the previous one. The timer only dispatches; the write is
        // a separate task, so nothing here is dropped from inside itself.
        self.save.debounce_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                let snapshot = this.snapshot(cx);
                if let Some(write) = this.spawn_write(snapshot, cx) {
                    this.save.write_task = Some(write);
                }
            })
            .ok();
        }));
    }

    /// Writes any pending change now and returns the write. Hold the task until it resolves:
    /// dropping it cancels the write.
    pub fn flush(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.save.debounce_task = None;
        let snapshot = self.snapshot(cx);
        self.spawn_write(snapshot, cx)
            .unwrap_or_else(|| Task::ready(()))
    }

    /// The app-quit hook: writes the tabs as they are now, and the quit waits for it. The future
    /// awaits the store directly (no task of ours), so it completes inside the quit's bounded
    /// wait.
    pub(super) fn on_quit(&mut self, cx: &mut Context<Self>) -> impl Future<Output = ()> + use<> {
        self.save.debounce_task = None;
        let snapshot = self.snapshot(cx);
        // Nothing happened since the start (leave what is stored alone: session restore reads
        // it), or what is stored is already this and no write is still in flight.
        let untouched = !self.save.dirty && self.save.last_written.is_none();
        let written =
            self.save.last_written.as_ref() == Some(&snapshot) && self.save.write_task.is_none();
        let skip = untouched || written;
        let store = self.store.clone();
        async move {
            if skip {
                return;
            }
            if let Err(error) = store.save(&snapshot).await {
                tracing::warn!(%error, "saving the cluster tabs on quit failed");
            }
        }
    }

    /// Starts writing `snapshot` unless it equals what was last written.
    fn spawn_write(&mut self, snapshot: SavedTabs, cx: &mut Context<Self>) -> Option<Task<()>> {
        if !self.save.dirty || self.save.last_written.as_ref() == Some(&snapshot) {
            self.save.dirty = false;
            return None;
        }
        self.save.last_written = Some(snapshot.clone());
        self.save.dirty = false;
        let store = self.store.clone();
        Some(cx.spawn(async move |this, cx| {
            if let Err(error) = store.save(&snapshot).await {
                tracing::warn!(%error, "saving the cluster tabs failed");
                // Not written: let the next change (or flush) try again.
                this.update(cx, |this, _| {
                    this.save.last_written = None;
                    this.save.dirty = true;
                })
                .ok();
            }
        }))
    }
}
