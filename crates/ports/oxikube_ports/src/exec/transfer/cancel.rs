//! [`CancellationToken`]: a runtime-free "stop now" shared by a transfer and whoever started it.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;

/// Asks a running transfer to stop. Cloning shares the token: cancel any clone and every holder
/// sees it.
///
/// It needs no async runtime (this crate has none): [`cancelled`](Self::cancelled) is a plain
/// future that wakes its task when the token is cancelled, so a transfer can wait on its data
/// and on the token at once. A transfer that sees the token cancelled drops its exec session,
/// which closes the connection and stops the remote process.
///
/// The same token serves every long-running exec helper (download, upload, listing) so a view
/// keeps one per operation and cancels it from its "Cancel" button or when it closes.
#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    waiters: Mutex<Waiters>,
}

#[derive(Default)]
struct Waiters {
    next_id: u64,
    wakers: Vec<(u64, Waker)>,
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancellationToken {
    /// A token nobody has cancelled yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels the token and wakes every task waiting on [`cancelled`](Self::cancelled).
    /// Idempotent.
    pub fn cancel(&self) {
        if self.inner.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        let wakers = std::mem::take(&mut self.inner.waiters.lock().wakers);
        for (_, waker) in wakers {
            waker.wake();
        }
    }

    /// Whether [`cancel`](Self::cancel) was called on this token or a clone.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// A future that resolves once the token is cancelled (at once if it already is). It owns a
    /// clone of the token, so it can be stored next to the work it guards.
    pub fn cancelled(&self) -> CancelledFuture {
        CancelledFuture {
            token: self.clone(),
            id: None,
        }
    }
}

/// Resolves when its [`CancellationToken`] is cancelled. See
/// [`CancellationToken::cancelled`].
#[must_use = "a future does nothing unless polled"]
pub struct CancelledFuture {
    token: CancellationToken,
    /// This future's slot among the token's wakers, once it waited.
    id: Option<u64>,
}

impl std::fmt::Debug for CancelledFuture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelledFuture")
            .field("cancelled", &self.token.is_cancelled())
            .finish()
    }
}

impl Future for CancelledFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if this.token.is_cancelled() {
            return Poll::Ready(());
        }
        let mut waiters = this.token.inner.waiters.lock();
        // `cancel` sets the flag before it takes the wakers under this lock: checking again here
        // means a cancel either sees this waker or this poll sees the flag.
        if this.token.is_cancelled() {
            return Poll::Ready(());
        }
        let id = match this.id {
            Some(id) => id,
            None => {
                let id = waiters.next_id;
                waiters.next_id += 1;
                this.id = Some(id);
                id
            }
        };
        match waiters.wakers.iter_mut().find(|(slot, _)| *slot == id) {
            Some((_, waker)) => waker.clone_from(cx.waker()),
            None => waiters.wakers.push((id, cx.waker().clone())),
        }
        Poll::Pending
    }
}

impl Drop for CancelledFuture {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.token
                .inner
                .waiters
                .lock()
                .wakers
                .retain(|(slot, _)| *slot != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;

    use futures::FutureExt as _;

    use super::*;

    struct CountingWaker(AtomicUsize);

    impl Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn clones_share_the_cancel_and_waiters_are_woken() {
        let token = CancellationToken::new();
        let counter = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);
        let mut wait = token.cancelled();
        assert!(Pin::new(&mut wait).poll(&mut cx).is_pending());
        // Polling again replaces the waker instead of piling them up.
        assert!(Pin::new(&mut wait).poll(&mut cx).is_pending());
        assert_eq!(token.inner.waiters.lock().wakers.len(), 1);

        let clone = token.clone();
        clone.cancel();
        assert!(token.is_cancelled());
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
        assert!(Pin::new(&mut wait).poll(&mut cx).is_ready());
        token.cancel();
        assert_eq!(counter.0.load(Ordering::SeqCst), 1, "cancel is idempotent");
        assert!(token.cancelled().now_or_never().is_some());
    }

    #[test]
    fn a_dropped_waiter_leaves_no_waker_behind() {
        let token = CancellationToken::new();
        let waker = Waker::from(Arc::new(CountingWaker(AtomicUsize::new(0))));
        let mut cx = Context::from_waker(&waker);
        let mut first = token.cancelled();
        let mut second = token.cancelled();
        assert!(Pin::new(&mut first).poll(&mut cx).is_pending());
        assert!(Pin::new(&mut second).poll(&mut cx).is_pending());
        drop(first);
        assert_eq!(token.inner.waiters.lock().wakers.len(), 1);
        drop(second);
        assert!(token.inner.waiters.lock().wakers.is_empty());
        assert!(!token.is_cancelled());
    }
}
