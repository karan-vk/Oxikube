//! Bounded channels from tokio producers to GPUI entities, drained in batches.
//!
//! A feed task (running under [`crate::spawn_kube`]) pushes items into a [`BatchSender`]; the view
//! hands the [`BatchReceiver`] to [`BatchReceiver::drain_into`], which spawns a foreground task that
//! wakes when items arrive, takes everything queued (up to a batch limit) in one go and calls the
//! view's handler once per batch. The UI thread never polls, and a burst of deltas costs one entity
//! update instead of one per item. Pair the handler with [`crate::notify_coalesced`] so the redraws
//! are capped at frame cadence too.
//!
//! ```ignore
//! let (tx, rx) = batch_channel::<Delta>(DEFAULT_CHANNEL_CAPACITY);
//! self.feed = spawn_kube(cx, async move { pump_watch_into(tx).await });
//! self.drain = rx.drain_into(cx, |this, batch, cx| {
//!     this.store.apply(batch);
//!     cx.notify_coalesced();
//! });
//! ```
//!
//! Backpressure: the channel is bounded, so a producer that outruns the UI waits in `send().await`
//! (on tokio, never on the UI thread). Both tasks are owned: dropping the drain task stops
//! draining and dropping the receiver closes the channel, which ends the producer's sends.

use gpui::{Context, Task};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context as PollContext, Poll};
use tokio::sync::mpsc;

pub use tokio::sync::mpsc::Sender as BatchSender;
pub use tokio::sync::mpsc::error::{SendError, TrySendError};

/// A sensible channel capacity for feeds: about a second of a busy watch.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 4096;

/// Most items handed to one handler call; the drain task yields to other foreground work between
/// batches so a flood cannot starve input or rendering.
pub const DEFAULT_BATCH_LIMIT: usize = 1024;

/// The receiving half of [`batch_channel`].
pub struct BatchReceiver<T> {
    rx: mpsc::Receiver<T>,
    batch_limit: usize,
}

/// A bounded channel of `capacity` items (at least 1) whose receiver drains into a GPUI entity.
pub fn batch_channel<T: Send + 'static>(capacity: usize) -> (BatchSender<T>, BatchReceiver<T>) {
    let (tx, rx) = mpsc::channel(capacity.max(1));
    (
        tx,
        BatchReceiver {
            rx,
            batch_limit: DEFAULT_BATCH_LIMIT,
        },
    )
}

impl<T: 'static> BatchReceiver<T> {
    /// Caps each batch at `limit` items (at least 1) instead of [`DEFAULT_BATCH_LIMIT`].
    #[must_use]
    pub fn with_batch_limit(mut self, limit: usize) -> Self {
        self.batch_limit = limit.max(1);
        self
    }

    /// Spawns the foreground task that drains this channel into the entity behind `cx`, calling
    /// `on_batch` once per batch of queued items (in send order; the batch buffer is reused).
    ///
    /// The task ends when every sender is dropped and the channel is empty, or when the entity is
    /// released. Store the returned task in the entity: dropping it stops the drain.
    pub fn drain_into<E: 'static>(
        self,
        cx: &mut Context<E>,
        mut on_batch: impl FnMut(&mut E, std::vec::Drain<'_, T>, &mut Context<E>) + 'static,
    ) -> Task<()> {
        let Self {
            mut rx,
            batch_limit,
        } = self;
        cx.spawn(async move |this, cx| {
            let mut buffer = Vec::with_capacity(batch_limit.min(DEFAULT_BATCH_LIMIT));
            loop {
                // 0 means closed and empty: every sender is gone.
                if rx.recv_many(&mut buffer, batch_limit).await == 0 {
                    break;
                }
                let delivered =
                    this.update(cx, |entity, cx| on_batch(entity, buffer.drain(..), cx));
                if delivered.is_err() {
                    break;
                }
                // `recv_many` is immediately ready while items are queued; without this a flood
                // would keep the main thread in this loop.
                YieldNow(false).await;
            }
        })
    }
}

/// Returns `Pending` once (waking itself), so the executor runs other queued work first.
struct YieldNow(bool);

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut PollContext<'_>) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}
