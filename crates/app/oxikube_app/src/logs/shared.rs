//! [`Shared`]: the state one session's driver task writes and its readers read.
//!
//! One short critical section per commit (the driver builds its entries first) and per read (the
//! UI copies the visible rows out). Waking is done after the lock is released.

use std::sync::atomic::{AtomicU64, Ordering};
use std::task::Waker;

use oxikube_ports::LogOptions;
use parking_lot::Mutex;

use super::entry::LogEntry;
use super::ring::LogBuffer;
use super::state::{EndReason, LogState};
use super::target::LogTarget;

/// What a session's driver and readers share.
pub(super) struct Shared {
    pub(super) id: u64,
    pub(super) target: LogTarget,
    pub(super) options: LogOptions,
    /// Batches committed so far.
    batches: AtomicU64,
    inner: Mutex<Inner>,
}

struct Inner {
    buffer: LogBuffer,
    state: LogState,
    /// The owner dropped the session: streams end.
    closed: bool,
    /// Streams waiting for a change; woken and cleared by every change.
    wakers: Vec<Waker>,
}

/// A consistent view of the shared state for one delta computation.
pub(super) struct Snapshot {
    pub first_seq: u64,
    pub next_seq: u64,
    pub state: LogState,
    pub closed: bool,
}

impl Shared {
    pub(super) fn new(id: u64, target: LogTarget, options: LogOptions, capacity: usize) -> Self {
        Self {
            id,
            target,
            options,
            batches: AtomicU64::new(0),
            inner: Mutex::new(Inner {
                buffer: LogBuffer::new(capacity),
                state: LogState::Connecting,
                closed: false,
                wakers: Vec::new(),
            }),
        }
    }

    /// Runs `f` over the buffer and the state, under the lock: keep it short.
    pub(super) fn read<R>(&self, f: impl FnOnce(&LogBuffer, &LogState) -> R) -> R {
        let inner = self.inner.lock();
        f(&inner.buffer, &inner.state)
    }

    pub(super) fn state(&self) -> LogState {
        self.inner.lock().state.clone()
    }

    /// Batches committed so far.
    pub(super) fn batches(&self) -> u64 {
        self.batches.load(Ordering::Relaxed)
    }

    /// Appends one batch, at `capacity` lines of room.
    pub(super) fn commit(&self, entries: Vec<LogEntry>, capacity: usize) {
        self.batches.fetch_add(1, Ordering::Relaxed);
        let wakers = {
            let mut inner = self.inner.lock();
            if inner.buffer.capacity() != capacity {
                inner.buffer.set_capacity(capacity);
            }
            inner.buffer.extend(entries);
            std::mem::take(&mut inner.wakers)
        };
        wake(wakers);
    }

    /// Moves to `state` unless the session already reached a terminal one.
    pub(super) fn set_state(&self, state: LogState) {
        let wakers = {
            let mut inner = self.inner.lock();
            if inner.state.is_terminal() || inner.state == state {
                return;
            }
            inner.state = state;
            std::mem::take(&mut inner.wakers)
        };
        wake(wakers);
    }

    /// Applies a new `logs.buffer_lines`.
    pub(super) fn set_capacity(&self, capacity: usize) {
        let wakers = {
            let mut inner = self.inner.lock();
            if inner.buffer.capacity() == capacity {
                return;
            }
            inner.buffer.set_capacity(capacity);
            std::mem::take(&mut inner.wakers)
        };
        wake(wakers);
    }

    /// The owner dropped the session: cancel what is not finished and end the streams.
    pub(super) fn close(&self) {
        let wakers = {
            let mut inner = self.inner.lock();
            inner.closed = true;
            if !inner.state.is_terminal() {
                inner.state = LogState::Ended(EndReason::Cancelled);
            }
            std::mem::take(&mut inner.wakers)
        };
        wake(wakers);
    }

    /// Whether the owner dropped the session.
    pub(super) fn is_closed(&self) -> bool {
        self.inner.lock().closed
    }

    /// The numbers a stream computes its next delta from; registers `waker` to be woken by the
    /// next change when `register` says the caller found nothing new.
    pub(super) fn snapshot(
        &self,
        waker: Option<&Waker>,
        is_new: impl FnOnce(&Snapshot) -> bool,
    ) -> Option<Snapshot> {
        let mut inner = self.inner.lock();
        let snapshot = Snapshot {
            first_seq: inner.buffer.first_seq(),
            next_seq: inner.buffer.next_seq(),
            state: inner.state.clone(),
            closed: inner.closed,
        };
        if is_new(&snapshot) {
            return Some(snapshot);
        }
        if let Some(waker) = waker
            && !inner.wakers.iter().any(|w| w.will_wake(waker))
        {
            inner.wakers.push(waker.clone());
        }
        None
    }
}

fn wake(wakers: Vec<Waker>) {
    for waker in wakers {
        waker.wake();
    }
}
