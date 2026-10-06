//! [`LogDelta`] and [`LogDeltas`]: what changed in a session since the consumer last looked.
//!
//! A delta is computed when the consumer polls, from where its cursor was, not queued per line:
//! a consumer that polls once per frame gets one delta per frame however many batches the driver
//! committed in between, and one that falls behind gets one larger delta, never an unbounded
//! queue. Each [`LogDeltas`] has its own cursor, so several views of one session do not steal
//! from each other.

use std::ops::Range;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;

use super::shared::{Shared, Snapshot};
use super::state::LogState;

/// One change of a session's buffer, relative to the previous delta of the same [`LogDeltas`].
///
/// With `len` the number of lines the consumer holds (the previous window), the new window holds
/// `len - dropped_front + appended.len()` lines, and it is exactly the session's lines with seq
/// `first_seq..appended.end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogDelta {
    /// Seqs of the lines appended since the previous delta. Empty when only the state changed or
    /// the buffer shrank. Lines that arrived and were dropped again before the consumer polled
    /// are not in it.
    pub appended: Range<u64>,
    /// Lines the previous window lost from its front (the oldest were dropped to stay within
    /// `logs.buffer_lines`).
    pub dropped_front: usize,
    /// Seq of the oldest line retained after this delta. Lines are numbered from 0 and dropped
    /// oldest first, so it is also how many lines were dropped over the session's life: above 0
    /// is the "truncated" marker.
    pub first_seq: u64,
    /// The session's state after this delta.
    pub state: LogState,
}

/// Where a consumer is: the window it has seen and the state it was told.
struct Cursor {
    first: u64,
    next: u64,
    state: LogState,
}

/// The deltas of one session; see [`LogSession::deltas`](super::LogSession::deltas). Ends after
/// the delta that carries a terminal state (`Ended` or `Failed`), or when the session is dropped.
/// Dropping the stream stops nothing: only dropping the session cancels the read.
pub struct LogDeltas {
    shared: Arc<Shared>,
    cursor: Cursor,
    done: bool,
}

impl Cursor {
    /// Whether `now` shows nothing the consumer has not seen.
    fn has_seen(&self, now: &Snapshot) -> bool {
        now.next_seq == self.next && now.first_seq == self.first && now.state == self.state
    }
}

impl LogDeltas {
    pub(super) fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            cursor: Cursor {
                first: 0,
                next: 0,
                state: LogState::Connecting,
            },
            done: false,
        }
    }

    fn delta(&mut self, now: Snapshot) -> LogDelta {
        let cursor = &mut self.cursor;
        let held = cursor.next - cursor.first;
        let dropped_front = (now.first_seq.saturating_sub(cursor.first)).min(held);
        let delta = LogDelta {
            appended: cursor.next.max(now.first_seq)..now.next_seq,
            dropped_front: usize::try_from(dropped_front).unwrap_or(usize::MAX),
            first_seq: now.first_seq,
            state: now.state.clone(),
        };
        *cursor = Cursor {
            first: now.first_seq,
            next: now.next_seq,
            state: now.state,
        };
        delta
    }
}

impl Stream for LogDeltas {
    type Item = LogDelta;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<LogDelta>> {
        let this = &mut *self;
        if this.done {
            return Poll::Ready(None);
        }
        let cursor = &this.cursor;
        let found = this
            .shared
            .snapshot(Some(cx.waker()), |now| now.closed || !cursor.has_seen(now));
        let Some(now) = found else {
            return Poll::Pending;
        };
        let closed = now.closed;
        if closed && this.cursor.has_seen(&now) {
            // The session went away after everything was delivered.
            this.done = true;
            return Poll::Ready(None);
        }
        let delta = this.delta(now);
        this.done = delta.state.is_terminal() || closed;
        Poll::Ready(Some(delta))
    }
}
