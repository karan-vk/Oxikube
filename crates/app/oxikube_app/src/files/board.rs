//! [`TransferBoard`]: the transfers of this run, running and recent, for the transfers panel.
//!
//! The service registers a transfer when it starts, updates its progress through a throttled
//! callback (at most ten a second, by the transfer helpers' meter), and settles it when it ends.
//! The panel reads [`snapshot`](TransferBoard::snapshot) at frame cadence, only while something
//! runs ([`version`](TransferBoard::version) tells whether anything changed). Nothing here is
//! persisted: it holds names and byte counts for this run only, never file content.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use oxikube_ports::exec::ProgressFn;
use oxikube_ports::{CancellationToken, TransferProgress};
use parking_lot::Mutex;

/// How long a transfer runs before its speed is worked out, in seconds.
const MIN_SPEED_SECONDS: f64 = 0.5;

/// How many finished transfers are kept for the panel before the oldest are forgotten.
const KEEP_FINISHED: usize = 30;

/// Names a transfer on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransferId(u64);

impl TransferId {
    /// The number the board gave the transfer (it names the transfer in toast keys and tests).
    pub fn number(self) -> u64 {
        self.0
    }
}

/// Which way a transfer goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From the container to the local disk.
    Download,
    /// From the local disk into the container.
    Upload,
}

/// Where a transfer is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferState {
    /// Moving bytes.
    Running,
    /// Finished.
    Done,
    /// Stopped by the user; nothing is left under the final name.
    Cancelled,
    /// Failed, with the message to show (for example `tar is not available in container app`).
    Failed(String),
}

impl TransferState {
    /// Whether the transfer is over.
    pub fn is_finished(&self) -> bool {
        !matches!(self, TransferState::Running)
    }
}

/// What the panel shows for one transfer.
#[derive(Debug, Clone, PartialEq)]
pub struct TransferInfo {
    /// Its name on the board.
    pub id: TransferId,
    /// The way it goes.
    pub direction: Direction,
    /// The pod and container, `ns/pod (container)`.
    pub place: String,
    /// The path in the container.
    pub remote: String,
    /// The local file or directory (its name only).
    pub local: String,
    /// Bytes moved.
    pub bytes: u64,
    /// The size expected, when known.
    pub total: Option<u64>,
    /// Where it is.
    pub state: TransferState,
    /// Bytes per second since it started (the mean over its whole run); 0 for its first half
    /// second.
    pub speed: f64,
    /// A note for the user once it is done (entries that were not unpacked, for instance).
    pub note: Option<String>,
}

impl TransferInfo {
    /// The fraction done in `0.0..=1.0`, when the total is known.
    pub fn fraction(&self) -> Option<f64> {
        TransferProgress {
            bytes: self.bytes,
            total: self.total,
        }
        .fraction()
    }
}

struct Slot {
    info: TransferInfo,
    token: CancellationToken,
    started: Instant,
    ended: Option<Duration>,
}

#[derive(Default)]
struct Inner {
    slots: Mutex<Vec<Slot>>,
    next: AtomicU64,
    version: AtomicU64,
}

/// The transfers of this run. Cheap to clone; every clone sees the same list.
#[derive(Clone, Default)]
pub struct TransferBoard {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for TransferBoard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferBoard")
            .field("transfers", &self.inner.slots.lock().len())
            .finish()
    }
}

/// What a transfer needs to report itself.
pub struct TransferHandle {
    /// Its name on the board.
    pub id: TransferId,
    /// Cancelling it stops the transfer.
    pub cancel: CancellationToken,
    /// Where the transfer helpers send progress.
    pub progress: ProgressFn,
}

/// Settles a transfer on the board when it is dropped without having ended: a transfer whose task
/// was aborted (the tab closed, the app quitting) must not stay "running" for ever.
pub struct SettleOnDrop {
    board: TransferBoard,
    id: TransferId,
    cancel: CancellationToken,
    settled: bool,
}

impl SettleOnDrop {
    /// Watches the transfer `handle` names on `board`.
    pub fn new(board: &TransferBoard, handle: &TransferHandle) -> Self {
        Self {
            board: board.clone(),
            id: handle.id,
            cancel: handle.cancel.clone(),
            settled: false,
        }
    }

    /// Settles the transfer with its outcome (see [`TransferBoard::finish`]).
    pub fn finish(mut self, result: Result<Option<String>, String>) {
        self.settled = true;
        self.board.finish(self.id, result);
    }
}

impl Drop for SettleOnDrop {
    fn drop(&mut self) {
        if !self.settled {
            // Dropped mid-flight: the exec session goes with it, so the transfer is stopped.
            self.cancel.cancel();
            self.board
                .finish(self.id, Err("the transfer was interrupted".to_owned()));
        }
    }
}

/// What the board is told when a transfer starts.
#[derive(Debug, Clone)]
pub struct TransferStart {
    /// The way it goes.
    pub direction: Direction,
    /// The pod and container.
    pub place: String,
    /// The path in the container.
    pub remote: String,
    /// The local name.
    pub local: String,
    /// The size expected, when known.
    pub total: Option<u64>,
}

impl TransferBoard {
    /// An empty board.
    pub fn new() -> Self {
        Self::default()
    }

    fn bump(&self) {
        self.inner.version.fetch_add(1, Ordering::Release);
    }

    /// A number that changes whenever the board does: compare it to skip a redraw.
    pub fn version(&self) -> u64 {
        self.inner.version.load(Ordering::Acquire)
    }

    /// Registers a running transfer.
    pub fn begin(&self, start: TransferStart) -> TransferHandle {
        let id = TransferId(self.inner.next.fetch_add(1, Ordering::Relaxed));
        let token = CancellationToken::new();
        let info = TransferInfo {
            id,
            direction: start.direction,
            place: start.place,
            remote: start.remote,
            local: start.local,
            bytes: 0,
            total: start.total,
            state: TransferState::Running,
            speed: 0.0,
            note: None,
        };
        {
            let mut slots = self.inner.slots.lock();
            slots.push(Slot {
                info,
                token: token.clone(),
                started: Instant::now(),
                ended: None,
            });
            let finished = slots.iter().filter(|s| s.info.state.is_finished()).count();
            if finished > KEEP_FINISHED {
                let mut drop_count = finished - KEEP_FINISHED;
                slots.retain(|slot| {
                    if drop_count > 0 && slot.info.state.is_finished() {
                        drop_count -= 1;
                        false
                    } else {
                        true
                    }
                });
            }
        }
        self.bump();
        let board = self.clone();
        let progress: ProgressFn = Arc::new(move |progress: TransferProgress| {
            board.advance(id, progress);
        });
        TransferHandle {
            id,
            cancel: token,
            progress,
        }
    }

    fn advance(&self, id: TransferId, progress: TransferProgress) {
        {
            let mut slots = self.inner.slots.lock();
            let Some(slot) = slots.iter_mut().find(|s| s.info.id == id) else {
                return;
            };
            slot.info.bytes = progress.bytes;
            if progress.total.is_some() {
                slot.info.total = progress.total;
            }
        }
        self.bump();
    }

    /// Settles a transfer: `Ok(note)` is done, an `Err` is a failure, unless the transfer was
    /// cancelled, which is then `Cancelled` whatever error the stop caused.
    pub fn finish(&self, id: TransferId, result: Result<Option<String>, String>) {
        {
            let mut slots = self.inner.slots.lock();
            let Some(slot) = slots.iter_mut().find(|s| s.info.id == id) else {
                return;
            };
            slot.ended = Some(slot.started.elapsed());
            match result {
                _ if slot.token.is_cancelled() => slot.info.state = TransferState::Cancelled,
                Ok(note) => {
                    slot.info.state = TransferState::Done;
                    slot.info.note = note;
                }
                Err(message) => slot.info.state = TransferState::Failed(message),
            }
        }
        self.bump();
    }

    /// Asks the transfer `id` to stop. `false` when it is not running.
    pub fn cancel(&self, id: TransferId) -> bool {
        let slots = self.inner.slots.lock();
        let Some(slot) = slots.iter().find(|s| s.info.id == id) else {
            return false;
        };
        if slot.info.state.is_finished() {
            return false;
        }
        slot.token.cancel();
        true
    }

    /// Forgets the finished transfers.
    pub fn clear_finished(&self) {
        self.inner
            .slots
            .lock()
            .retain(|slot| !slot.info.state.is_finished());
        self.bump();
    }

    /// How many transfers are running.
    pub fn running(&self) -> usize {
        self.inner
            .slots
            .lock()
            .iter()
            .filter(|s| !s.info.state.is_finished())
            .count()
    }

    /// The transfers, oldest first, with their speeds worked out now.
    pub fn snapshot(&self) -> Vec<TransferInfo> {
        self.inner
            .slots
            .lock()
            .iter()
            .map(|slot| {
                let elapsed = slot.ended.unwrap_or_else(|| slot.started.elapsed());
                let mut info = slot.info.clone();
                let seconds = elapsed.as_secs_f64();
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a speed shown to a few digits does not need the low bits"
                )]
                let bytes = info.bytes as f64;
                // A rate over a few milliseconds is noise: show none until it has some history.
                info.speed = if seconds >= MIN_SPEED_SECONDS {
                    bytes / seconds
                } else {
                    0.0
                };
                info
            })
            .collect()
    }
}
