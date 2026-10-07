//! The tokio side of the bridge: the output pump (backend → grid) and the writer
//! (keystrokes, replies and resizes → backend). Both are plain async functions: the entity runs
//! them through `oxikube_runtime::spawn_kube` (abort on drop), the tests drive them directly.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use bytes::{Bytes, BytesMut};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::stream::BoxStream;
use futures::{FutureExt as _, StreamExt as _};
use oxikube_domain::OxiError;
use oxikube_ports::{BackendEvent, ExitStatus, TerminalBackend, TerminalSize};
use parking_lot::Mutex;
use tokio::sync::{mpsc, watch};

use crate::grid::{GridEvent, TermGrid};

/// The most output parsed under one hold of the grid lock. A snapshot waiting for the lock waits
/// at most this much parsing (well under 0.1 ms in release builds), however large the chunk.
pub(crate) const PARSE_SLICE: usize = 16 * 1024;

/// Held by a search for its whole sliced scan (see `TerminalState::search`): the pump takes it
/// before parsing, so output waits (asynchronously, no thread blocked) instead of moving lines a
/// search is halfway through, and the grid lock itself is only ever held for one slice.
pub(crate) type OutputGate = Arc<futures::lock::Mutex<()>>;

/// What the pump tells the UI thread.
#[derive(Debug)]
pub(crate) enum GridUpdate {
    /// The grid changed. At most one is in flight: the pump sends it only when it flips the wake
    /// flag on, the UI turns the flag off when it handles it.
    Changed,
    /// Something the view acts on (title, bell, clipboard, colour query).
    Event(GridEvent),
    /// The process ended; the last update.
    Exited(ExitStatus),
    /// The transport failed.
    Error(OxiError),
}

/// Reads `events` until the session ends: parses output into `grid` in [`PARSE_SLICE`] pieces,
/// sends the emulator's replies to the writer through `replies`, and wakes the UI through
/// `updates` (once per wave of output, see [`GridUpdate::Changed`]). Parses only while it holds
/// `gate` (a search may be running).
pub(crate) async fn pump(
    mut events: BoxStream<'static, BackendEvent>,
    grid: Arc<Mutex<TermGrid>>,
    gate: OutputGate,
    replies: UnboundedSender<Bytes>,
    updates: mpsc::Sender<GridUpdate>,
    wake: Arc<AtomicBool>,
) {
    let mut grid_events = Vec::new();
    let mut sync_deadline: Option<Instant> = None;
    loop {
        let Some(event) = next_event(&mut events, sync_deadline).await else {
            // The deadline of a synchronized update passed with no more output: apply it.
            sync_deadline = {
                let _parsing = gate.lock().await;
                let mut grid = grid.lock();
                grid.flush_sync(&mut grid_events);
                grid.sync_deadline()
            };
            if !deliver(&mut grid_events, &replies, &updates, &wake).await {
                return;
            }
            continue;
        };
        match event {
            Some(BackendEvent::Output(bytes)) => {
                let parsing = gate.lock().await;
                for slice in bytes.chunks(PARSE_SLICE) {
                    let mut grid = grid.lock();
                    grid.advance(slice, &mut grid_events);
                    sync_deadline = grid.sync_deadline();
                }
                drop(parsing);
                oxikube_runtime::perf::record_feed_deltas(1);
                if !deliver(&mut grid_events, &replies, &updates, &wake).await {
                    return;
                }
            }
            Some(BackendEvent::Error(error)) => {
                if updates.send(GridUpdate::Error(error)).await.is_err() {
                    return;
                }
            }
            Some(BackendEvent::Exited(status)) => {
                let _ = updates.send(GridUpdate::Exited(status)).await;
                return;
            }
            // The stream ended without an exit status (a backend torn down under us).
            None => {
                let _ = updates
                    .send(GridUpdate::Exited(ExitStatus::default()))
                    .await;
                return;
            }
        }
    }
}

/// The next backend event (`Some(None)` when the stream ended), or `None` when `sync_deadline`
/// passes first. The deadline needs a tokio timer: without a tokio runtime (deterministic GPUI
/// tests) a pending update waits for the next output or its end sequence.
async fn next_event(
    events: &mut BoxStream<'static, BackendEvent>,
    sync_deadline: Option<Instant>,
) -> Option<Option<BackendEvent>> {
    match sync_deadline {
        Some(deadline) if tokio::runtime::Handle::try_current().is_ok() => {
            let deadline = tokio::time::Instant::from_std(deadline);
            tokio::time::timeout_at(deadline, events.next()).await.ok()
        }
        _ => Some(events.next().await),
    }
}

/// Routes the grid events of one parse: replies to the writer, the rest to the UI, then the wake.
/// `false` once the UI side is gone (the entity was dropped): the pump stops.
async fn deliver(
    grid_events: &mut Vec<GridEvent>,
    replies: &UnboundedSender<Bytes>,
    updates: &mpsc::Sender<GridUpdate>,
    wake: &AtomicBool,
) -> bool {
    for event in grid_events.drain(..) {
        match event {
            GridEvent::Reply(bytes) => {
                // The writer is gone only when the entity is: nothing left to answer.
                let _ = replies.unbounded_send(bytes);
            }
            event => {
                if updates.send(GridUpdate::Event(event)).await.is_err() {
                    return false;
                }
            }
        }
    }
    // Everything parsed so far is in the grid before the flag flips, so the snapshot the UI takes
    // after clearing it sees this output.
    if !wake.swap(true, Ordering::AcqRel) && updates.send(GridUpdate::Changed).await.is_err() {
        return false;
    }
    true
}

/// Sends input bytes and resizes to `backend` in order, until the entity drops its ends.
///
/// Input queued while a write is in flight is merged into the next write. Resizes coalesce: the
/// watch channel keeps only the latest size, so a window drag costs one backend resize per write
/// round trip, not one per layout. Failures are logged without their bytes (they may be secrets)
/// and do not stop the loop: the pump reports the session's end.
pub(crate) async fn write_loop(
    backend: Arc<dyn TerminalBackend>,
    mut input: UnboundedReceiver<Bytes>,
    mut resize: watch::Receiver<TerminalSize>,
) {
    let mut buffer = BytesMut::new();
    let mut resize_open = true;
    loop {
        tokio::select! {
            biased;
            changed = resize.changed(), if resize_open => {
                if changed.is_err() {
                    resize_open = false;
                    continue;
                }
                let size = *resize.borrow_and_update();
                if let Err(error) = backend.resize(size).await {
                    tracing::debug!(%error, "terminal resize failed");
                }
            }
            bytes = input.next() => {
                let Some(bytes) = bytes else { return };
                buffer.extend_from_slice(&bytes);
                while let Some(Some(more)) = input.next().now_or_never() {
                    buffer.extend_from_slice(&more);
                }
                if let Err(error) = backend.write(&buffer).await {
                    tracing::debug!(%error, bytes = buffer.len(), "terminal write failed");
                }
                buffer.clear();
            }
        }
    }
}
