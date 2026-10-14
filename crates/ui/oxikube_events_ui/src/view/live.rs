//! The live side of the view: the subscription's changes, the lists read off the UI thread, the
//! tail's pause, and the clock of the relative cells.

use std::pin::Pin;
use std::time::Duration;

use futures::Stream as _;
use gpui::{AppContext as _, Context};
use oxikube_app::events::EventsChanged;
use oxikube_runtime::NotifyCoalescedExt as _;

use super::EventsView;
use crate::model::{RowSet, Tail};

/// How often the Age and Last seen cells move on.
const CLOCK_TICK: Duration = Duration::from_secs(1);

impl EventsView {
    /// Polls the subscription for as long as the view lives. The service coalesces everything
    /// that changed between two polls into one item, so a burst of events is one apply and one
    /// coalesced redraw.
    pub(super) fn start_live(&mut self, cx: &mut Context<Self>) {
        let subscription = self.subscription.clone();
        let task = cx.spawn(async move |this, cx| {
            loop {
                // The borrow lasts one poll: the view changes the filter between polls.
                let next = futures::future::poll_fn(|task_cx| {
                    Pin::new(&mut *subscription.borrow_mut()).poll_next(task_cx)
                })
                .await;
                let Some(changed) = next else { break };
                if this
                    .update(cx, |view, cx| view.on_changed(changed, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        self.tasks.push(task);
    }

    /// One coalesced change from the service.
    fn on_changed(&mut self, changed: EventsChanged, cx: &mut Context<Self>) {
        if let Some(state) = &changed.state {
            self.feed = state.clone();
            cx.notify_coalesced();
        }
        if changed.reset {
            // A reset carries the state only when it changed: read it, it may be older news.
            let state = self.subscription.borrow().state();
            if state != self.feed {
                self.feed = state;
                cx.notify_coalesced();
            }
            // A new filter shows its rows at once; a relist of the feed waits for the user's
            // resume like any other change.
            if self.model(cx).is_paused() && !std::mem::take(&mut self.refiltered) {
                self.table.update_quiet(cx, |d| d.model.mark_stale());
                cx.notify_coalesced();
                return;
            }
            self.refiltered = false;
            self.read_list(cx);
            return;
        }
        if self.reading {
            self.held.push(changed);
            return;
        }
        let paused = self.model(cx).is_paused();
        let moved = self.table.update_quiet(cx, |d| d.model.apply(&changed));
        if moved || paused {
            cx.notify_coalesced();
        }
    }

    /// Reads the matching events again and turns them into rows on the background executor; the
    /// changes that arrive meanwhile wait and are applied on top. A newer read drops an older one.
    pub(crate) fn read_list(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.reading = true;
        self.held.clear();
        let service = self.deps.service.clone();
        let cluster = self.cluster.clone();
        let filter = self.applied.clone();
        let (sort, first_id) = {
            let model = self.model(cx);
            (model.sort(), model.next_id())
        };
        let read = cx.background_spawn(async move {
            RowSet::build(service.query(&cluster, &filter), sort, first_id)
        });
        // Replaced (and so cancelled) by the next read, never cleared from inside.
        self.read_task = Some(cx.spawn(async move |this, cx| {
            let set = read.await;
            this.update(cx, |view, cx| view.install(generation, set, cx))
                .ok();
        }));
    }

    fn install(&mut self, generation: u64, set: RowSet, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        self.reading = false;
        let held = std::mem::take(&mut self.held);
        self.table.update_quiet(cx, |d| {
            d.model.install(set);
            for changed in &held {
                d.model.apply(changed);
            }
        });
        cx.notify();
    }

    /// `events::TogglePause`: pauses the tail, or resumes it (reading the list again when
    /// something changed meanwhile) and goes back to the top.
    pub fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if self.model(cx).is_paused() {
            self.resume(cx);
            self.table.scroll_to_row(0, cx);
        } else {
            self.table.update_quiet(cx, |d| d.model.pause(false));
        }
        cx.notify();
    }

    /// Scrolls `row` to the top of the table (a scroll away from the top pauses the tail).
    pub fn scroll_to(&mut self, row: usize, cx: &mut Context<Self>) {
        self.table.scroll_to_row(row, cx);
    }

    fn resume(&mut self, cx: &mut Context<Self>) {
        if self.table.update_quiet(cx, |d| d.model.resume()) {
            self.read_list(cx);
        }
    }

    /// The first visible row changed (a scroll): away from the top pauses the tail so rows do
    /// not move under the pointer; back at the top resumes a tail that paused that way.
    pub(crate) fn on_scrolled(&mut self, first_visible: usize, cx: &mut Context<Self>) {
        let tail = self.model(cx).tail();
        match (first_visible > 0, tail) {
            (true, Tail::Live) => {
                self.table.update_quiet(cx, |d| d.model.pause(true));
                cx.notify();
            }
            (false, Tail::Paused { auto: true }) => {
                self.resume(cx);
                cx.notify();
            }
            _ => {}
        }
    }

    /// Moves the relative cells on once a second while the view is the displayed tab.
    pub(super) fn start_clock(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK_TICK).await;
                let alive = this.update(cx, |view, cx| {
                    if view.active {
                        view.tick(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        self.tasks.push(task);
    }

    /// Moves the relative cells to the clock's now.
    pub(super) fn tick(&mut self, cx: &mut Context<Self>) {
        let now = self.deps.clock.now();
        self.table.update_quiet(cx, |d| d.tick(now));
        cx.notify_coalesced();
    }
}
