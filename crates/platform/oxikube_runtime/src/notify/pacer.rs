//! The frame pacer behind [`super::notify_coalesced`]: the pending set, the windows' next-frame
//! hooks and the backstop timer. One GPUI global, UI thread only.
//!
//! A **batch** is the set of entities marked since the last delivery. The first mark of a batch
//! asks every window for its next frame (`Window::on_next_frame`, which GPUI runs at the start of
//! the frame, before it draws) and arms a backstop timer. Whichever comes first delivers the batch:
//! one `notify` per entity, then the set is empty and the next mark starts the next batch.
//!
//! - **Frame.** The normal path: the display link's frame request runs the hook, the hook notifies
//!   the batch and GPUI draws it in that same frame. So a view hears at most one coalesced notify per
//!   drawn frame at any refresh rate, and an event that arrives right after a frame waits for the
//!   next frame only, not for a timer.
//! - **Backstop.** No frame comes when there is no window (headless, tests, start-up) or when the
//!   windows are not presenting (macOS stops the display link of a window nobody can see). The timer
//!   then delivers the batch, so models and their observers keep up. It waits [`FRAME_INTERVAL`]
//!   when no frame hook has run within [`FRAME_STALL`] (nothing is presenting) and [`FRAME_STALL`]
//!   while frames are flowing, so a late frame is waited for rather than raced.
//! - **One per frame, even then.** A backstop delivery sets `delivered_off_frame`; the next frame
//!   hook clears it and leaves the batch for the frame after, because the frame it starts already
//!   draws what the backstop delivered.
//!
//! Each window has at most one hook queued (`armed`): a window that is not presenting keeps its one
//! hook until it presents again instead of collecting one per batch.

use crate::perf;
use gpui::{AnyWeakEntity, App, EntityId, Global, Window, WindowId};
use std::collections::HashMap;
use std::mem;
use std::time::{Duration, Instant};

/// One frame at 120 Hz, the reference display (ADR 0013): the backstop's delay when no window is
/// presenting frames (no window, headless runs, tests), where it is the coalescing cadence itself.
pub const FRAME_INTERVAL: Duration = Duration::from_micros(8_333);

/// How long a batch may wait for a frame while frames are flowing before the backstop delivers it,
/// and how recent the last frame hook must be for frames to count as flowing. Far above a late
/// frame and above the 30 fps GPUI paces an inactive window at, so the backstop only fires for a
/// window that stopped presenting.
pub const FRAME_STALL: Duration = Duration::from_millis(100);

/// The pacer's state (see the module docs).
#[derive(Default)]
pub(super) struct Pacer {
    /// The current batch: entities to notify at the next delivery.
    pending: HashMap<EntityId, AnyWeakEntity>,
    /// The previous batch's emptied map, reused so a delivery does not allocate.
    spare: HashMap<EntityId, AnyWeakEntity>,
    /// Deliveries so far; a backstop only delivers the batch it was armed for.
    delivered: u64,
    /// Windows with a frame hook queued.
    armed: Vec<WindowId>,
    /// When a frame hook last ran.
    last_frame: Option<Instant>,
    /// A backstop delivered since the last frame hook ran.
    delivered_off_frame: bool,
}

impl Global for Pacer {}

impl Pacer {
    /// Whether `entity_id` is in the current batch.
    pub(super) fn is_pending(&self, entity_id: EntityId) -> bool {
        self.pending.contains_key(&entity_id)
    }
}

/// Adds `entity` to the current batch; starts the batch if it is the first.
pub(super) fn mark(entity_id: EntityId, entity: AnyWeakEntity, cx: &mut App) {
    let pacer = cx.default_global::<Pacer>();
    let first = pacer.pending.is_empty();
    if pacer.pending.insert(entity_id, entity).is_some() {
        return;
    }
    if first {
        start_batch(cx);
    }
}

/// Asks the windows for their next frame and arms the backstop for the batch just started.
fn start_batch(cx: &mut App) {
    let now = cx.background_executor().now();
    let pacer = cx.global::<Pacer>();
    let batch = pacer.delivered;
    let frames_flowing = pacer
        .last_frame
        .is_some_and(|last| now.saturating_duration_since(last) < FRAME_STALL);
    let backstop = if frames_flowing {
        FRAME_STALL
    } else {
        FRAME_INTERVAL
    };
    let windows = cx.windows();
    if windows
        .iter()
        .any(|window| !pacer.armed.contains(&window.window_id()))
    {
        // Deferred: the caller may be inside one of the windows' updates, where that window cannot
        // be updated again. Effects flush once the outermost update returns the windows.
        cx.defer(arm_windows);
    }
    let timer = cx.background_executor().timer(backstop);
    cx.spawn(async move |cx| {
        timer.await;
        cx.update(|cx| deliver_from_backstop(batch, cx));
    })
    // Detached on purpose: it owns its completion (a stale batch is a no-op) and must outlive the
    // update that started the batch.
    .detach();
}

/// Queues a frame hook on every open window that has none.
fn arm_windows(cx: &mut App) {
    let windows = cx.windows();
    let pacer = cx.default_global::<Pacer>();
    // A closed window drops its queued hook with it.
    pacer
        .armed
        .retain(|armed| windows.iter().any(|window| window.window_id() == *armed));
    if pacer.pending.is_empty() {
        return;
    }
    for window in windows {
        let window_id = window.window_id();
        if cx.global::<Pacer>().armed.contains(&window_id) {
            continue;
        }
        let queued = window.update(cx, |_, window, _| arm(window_id, window));
        if queued.is_ok() {
            cx.global_mut::<Pacer>().armed.push(window_id);
        }
    }
}

/// Queues the frame hook on `window`.
fn arm(window_id: WindowId, window: &Window) {
    window.on_next_frame(move |window, cx| on_frame(window_id, window, cx));
}

/// The frame hook: runs at the start of a frame of `window`, before it draws.
fn on_frame(window_id: WindowId, window: &mut Window, cx: &mut App) {
    let now = cx.background_executor().now();
    let pacer = cx.default_global::<Pacer>();
    pacer.armed.retain(|armed| *armed != window_id);
    pacer.last_frame = Some(now);
    if mem::take(&mut pacer.delivered_off_frame) {
        // This frame draws what the backstop delivered: the current batch waits for the next one.
        if !pacer.pending.is_empty() {
            pacer.armed.push(window_id);
            arm(window_id, window);
        }
        return;
    }
    if !pacer.pending.is_empty() {
        deliver(cx);
    }
}

/// The backstop: delivers batch `batch` if no frame has.
fn deliver_from_backstop(batch: u64, cx: &mut App) {
    let pacer = cx.default_global::<Pacer>();
    if pacer.delivered != batch || pacer.pending.is_empty() {
        return;
    }
    // Only a hook still queued starts a frame that draws this delivery; with none queued, the next
    // hook belongs to a later batch and must deliver it.
    pacer.delivered_off_frame = !pacer.armed.is_empty();
    deliver(cx);
}

/// Notifies every live entity of the current batch once and empties it.
fn deliver(cx: &mut App) {
    let pacer = cx.global_mut::<Pacer>();
    pacer.delivered += 1;
    // Swap the batch out before notifying: observers run when this update flushes, and one that
    // marks an entity again starts the next batch.
    let spare = mem::take(&mut pacer.spare);
    let mut batch = mem::replace(&mut pacer.pending, spare);
    for (entity_id, entity) in batch.drain() {
        // A released entity has nothing to notify.
        if entity.is_upgradable() {
            cx.notify(entity_id);
            perf::record_view_notify(entity_id.as_u64());
        }
    }
    cx.global_mut::<Pacer>().spare = batch;
}
