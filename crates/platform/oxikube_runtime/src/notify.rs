//! [`notify_coalesced`]: batch `cx.notify()` to frame cadence.
//!
//! A feed can apply thousands of deltas per second. Calling `cx.notify()` per delta runs every
//! observer (and re-renders every dependent view) thousands of times for at most 120 visible frames.
//! `notify_coalesced` marks the entity dirty instead and schedules a single `cx.notify()`
//! [`FRAME_INTERVAL`] later; every further call before then is a hash-set lookup. Each delivered
//! notify is counted by [`crate::perf::record_notify`] (the `notifies` figure of `--perf`).
//!
//! Use it for streams (watches, logs, terminal bytes, agent tokens). Direct user input keeps using
//! `cx.notify()`: the coalesced notify lands up to one frame later, which is inside the input
//! budget only when nothing else is queued.
//!
//! The pending set is a GPUI global keyed by [`EntityId`], so the helper needs no field in the
//! entity and no `init`. The delivery task is detached (the flag + detach pattern of the crate
//! docs): it clears the flag itself, and only notifies if the entity is still alive.

use crate::perf;
use gpui::{Context, EntityId, Global};
use std::collections::HashSet;
use std::time::Duration;

/// How long a coalesced notify waits: one frame at 120 Hz, the reference display (ADR 0013).
/// At 60 Hz this delivers twice per frame, which GPUI folds into one redraw anyway.
pub const FRAME_INTERVAL: Duration = Duration::from_micros(8_333);

/// Entities with a coalesced notify in flight: one entry per streaming view, so a std set is plenty.
#[derive(Default)]
struct PendingNotifies(HashSet<EntityId>);

impl Global for PendingNotifies {}

/// Schedules one `cx.notify()` for this entity within [`FRAME_INTERVAL`]; repeated calls before it
/// fires are folded into it. See the module docs.
pub fn notify_coalesced<T: 'static>(cx: &mut Context<T>) {
    let entity_id = cx.entity_id();
    if !cx.default_global::<PendingNotifies>().0.insert(entity_id) {
        return;
    }
    let timer = cx.background_executor().timer(FRAME_INTERVAL);
    cx.spawn(async move |this, cx| {
        timer.await;
        // Clear the flag before notifying: observers run when this update flushes, and one that
        // calls `notify_coalesced` again must schedule the next frame's notify.
        let notified = this.update(cx, |_, cx| {
            cx.default_global::<PendingNotifies>().0.remove(&entity_id);
            cx.notify();
        });
        match notified {
            Ok(()) => perf::record_notify(),
            // The entity was released meanwhile: nothing to notify, just drop the flag.
            Err(_) => cx.update(|cx| {
                cx.default_global::<PendingNotifies>().0.remove(&entity_id);
            }),
        }
    })
    // Detached on purpose: the task owns its own completion (flag + detach), and it must outlive
    // the update that scheduled it.
    .detach();
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
    cx.try_global::<PendingNotifies>()
        .is_some_and(|pending| pending.0.contains(&entity_id))
}
