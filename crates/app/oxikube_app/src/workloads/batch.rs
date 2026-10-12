//! The per-object batch runner (E12-S03, reused by the bulk actions of E12-S19): runs one
//! operation over many objects with bounded concurrency and keeps the results in the order asked.
//!
//! `pod::Sanitize` ([`WorkloadService::delete_each`](super::WorkloadService::delete_each)) and
//! `selection::Delete`, `selection::Restart` and `selection::Label`
//! ([`crate::selection`]) all run through [`run_each`], so one place decides how many requests
//! are in flight. An operation never stops the others: it reports its own failure as its result.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt as _;
use futures::stream;

/// How many objects of a batch are worked on at once.
pub const CONCURRENCY: usize = 6;

/// Runs `op` over `items` with at most `concurrency` (at least one) running at once and returns
/// the results in the order of `items`.
pub async fn run_each<I, T, Fut>(items: Vec<I>, concurrency: usize, op: impl Fn(I) -> Fut) -> Vec<T>
where
    Fut: Future<Output = T>,
{
    stream::iter(items)
        .map(op)
        .buffered(concurrency.max(1))
        .collect()
        .await
}

/// A new id for a batch: unique within the process and across restarts, short enough to read in
/// an audit row (`sweep-1a2b3c-0`, `selection-1a2b3c-1`). Every audit record of the batch
/// carries it as `batch=<id>`.
pub fn new_batch_id(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{:x}-{n:x}", nanos & 0xffff_ffff_ffff)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn results_keep_the_order_asked() {
        let out =
            futures::executor::block_on(run_each(vec![3, 1, 2], 2, |n| async move { n * 10 }));
        assert_eq!(out, vec![30, 10, 20]);
    }

    #[test]
    fn no_more_than_the_limit_run_at_once() {
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (r, p) = (running, peak.clone());
        futures::executor::block_on(run_each((0..30).collect(), 4, move |_| {
            let (running, peak) = (r.clone(), p.clone());
            async move {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                // Yield so the others get to start: a runner that ignored its limit would show.
                for _ in 0..3 {
                    futures::future::ready(()).await;
                    yield_once().await;
                }
                running.fetch_sub(1, Ordering::SeqCst);
            }
        }));
        assert_eq!(peak.load(Ordering::SeqCst), 4);
    }

    /// Returns `Pending` once, waking itself.
    async fn yield_once() {
        let mut yielded = false;
        futures::future::poll_fn(|cx| {
            if yielded {
                std::task::Poll::Ready(())
            } else {
                yielded = true;
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        })
        .await;
    }

    #[test]
    fn a_zero_limit_still_runs_one_at_a_time() {
        let out = futures::executor::block_on(run_each(vec![1, 2], 0, |n| async move { n }));
        assert_eq!(out, vec![1, 2]);
    }

    #[test]
    fn batch_ids_are_unique_and_carry_their_prefix() {
        let (a, b) = (new_batch_id("selection"), new_batch_id("selection"));
        assert_ne!(a, b);
        assert!(a.starts_with("selection-"));
    }
}
