//! [`notify_coalesced`]: batch `cx.notify()` to the window's frames.
//!
//! A feed can apply thousands of deltas per second. Calling `cx.notify()` per delta runs every
//! observer (and re-renders every dependent view) thousands of times for at most 120 visible frames.
//! `notify_coalesced` marks the entity pending instead; every further call before the next frame is
//! a hash-map lookup. At the start of the next frame (GPUI's `Window::on_next_frame`, before the
//! window draws) each pending entity is notified once, so a view hears **at most one coalesced
//! notify per drawn frame** at any refresh rate (120 Hz, 60 Hz, ProMotion), and an event that
//! arrives right after a frame is drawn in the next frame with no timer in between (E05-P599).
//! When no frame comes (no window, a window macOS stopped presenting, headless tests) a backstop
//! timer delivers instead: after [`FRAME_INTERVAL`] when no window has presented within
//! [`FRAME_STALL`], after [`FRAME_STALL`] while frames are flowing. The details are in `pacer`.
//!
//! Each delivered notify is counted by [`crate::perf::record_view_notify`] (the `notifies` figure
//! of `--perf`, and per view `max_view_notifies_per_frame`).
//!
//! Use it for streams (watches, logs, terminal bytes, agent tokens). Direct user input keeps using
//! `cx.notify()`: the coalesced notify lands in the next frame, which is inside the input budget
//! only when nothing else is queued.
//!
//! The pending set is a GPUI global keyed by [`EntityId`], so the helper needs no field in the
//! entity and no `init`. Entities released before the delivery are skipped.

mod pacer;

pub use pacer::{FRAME_INTERVAL, FRAME_STALL};

use gpui::{Context, EntityId};

/// Marks this entity for one `cx.notify()` at the start of the next frame; repeated calls before
/// then are folded into it. See the module docs.
pub fn notify_coalesced<T: 'static>(cx: &mut Context<T>) {
    let entity_id = cx.entity_id();
    // The hot path: an entity already pending costs one lookup and no handle clone.
    if notify_pending(cx, entity_id) {
        return;
    }
    let entity = cx.weak_entity().into();
    pacer::mark(entity_id, entity, cx);
}

/// `cx.notify_coalesced()`: method form of [`notify_coalesced`].
pub trait NotifyCoalescedExt {
    /// See [`notify_coalesced`].
    fn notify_coalesced(&mut self);
}

impl<T: 'static> NotifyCoalescedExt for Context<'_, T> {
    fn notify_coalesced(&mut self) {
        notify_coalesced(self);
    }
}

/// Whether `entity_id` has a coalesced notify in flight (tests and diagnostics).
pub fn notify_pending(cx: &gpui::App, entity_id: EntityId) -> bool {
    cx.try_global::<pacer::Pacer>()
        .is_some_and(|pacer| pacer.is_pending(entity_id))
}
