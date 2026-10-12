//! Observing how many calls of a fake are in flight at once.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll};

/// Records the most calls of a fake that were in flight at the same time.
///
/// Off by default, so a fake answers at once. After [`track`](Self::track) a call counts as in
/// flight from [`enter`](Self::enter) until its guard drops, and yields to the executor once in
/// between, so that calls a caller runs concurrently overlap and a caller that runs them one at a
/// time does not. Only an executor that re-polls (`futures::executor::block_on`) can drive it.
#[derive(Debug, Clone, Default)]
pub struct Overlap {
    state: Arc<OverlapState>,
}

#[derive(Debug, Default)]
struct OverlapState {
    tracking: AtomicBool,
    running: AtomicUsize,
    peak: AtomicUsize,
}

impl Overlap {
    /// Starts counting, and making calls yield once.
    pub fn track(&self) {
        self.state.tracking.store(true, Ordering::SeqCst);
    }

    /// The most calls in flight at once since [`track`](Self::track).
    pub fn peak(&self) -> usize {
        self.state.peak.load(Ordering::SeqCst)
    }

    /// Marks a call as in flight until the guard drops and, when tracking, yields once.
    pub async fn enter(&self) -> InFlight {
        if !self.state.tracking.load(Ordering::SeqCst) {
            return InFlight(None);
        }
        let now = self.state.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.state.peak.fetch_max(now, Ordering::SeqCst);
        let guard = InFlight(Some(self.state.clone()));
        YieldOnce(false).await;
        guard
    }
}

/// A call in flight: leaves the count when dropped.
#[derive(Debug)]
pub struct InFlight(Option<Arc<OverlapState>>);

impl Drop for InFlight {
    fn drop(&mut self) {
        if let Some(state) = &self.0 {
            state.running.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use futures::executor::block_on;
    use futures::stream;

    use super::*;

    async fn call(overlap: &Overlap) {
        let _in_flight = overlap.enter().await;
    }

    #[test]
    fn untracked_calls_answer_at_once() {
        let overlap = Overlap::default();
        futures::FutureExt::now_or_never(call(&overlap)).expect("no yield");
        assert_eq!(overlap.peak(), 0);
    }

    #[test]
    fn the_peak_is_the_most_calls_at_once() {
        let overlap = Overlap::default();
        overlap.track();
        let run = |limit| {
            block_on(
                stream::iter(0..8)
                    .map(|_| call(&overlap))
                    .buffered(limit)
                    .collect::<Vec<_>>(),
            )
        };
        run(1);
        assert_eq!(overlap.peak(), 1);
        run(3);
        assert_eq!(overlap.peak(), 3);
    }
}
