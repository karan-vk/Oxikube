//! A deadline on the app clock, so a call that never answers cannot hold an agent's turn and a
//! test drives the wait without sleeping.

use std::future::Future;
use std::time::Duration;

use futures::future::{Either, select};
use oxikube_ports::ClockPort;

/// `work`'s output, or `None` when `limit` passes first (and `work` is dropped, which cancels
/// the request in flight).
pub(crate) async fn within<T>(
    clock: &dyn ClockPort,
    limit: Duration,
    work: impl Future<Output = T>,
) -> Option<T> {
    let work = std::pin::pin!(work);
    let timer = std::pin::pin!(clock.sleep(limit));
    match select(work, timer).await {
        Either::Left((output, _)) => Some(output),
        Either::Right(_) => None,
    }
}

/// [`within`] the `top_*` and `@metrics` deadline: how long a metrics read may take.
pub(crate) async fn within_snapshot_deadline<T>(
    clock: &dyn ClockPort,
    work: impl Future<Output = T>,
) -> Option<T> {
    within(clock, super::bounds::SNAPSHOT_DEADLINE, work).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use oxikube_testkit::FakeClockPort;

    use super::*;

    #[tokio::test]
    async fn work_that_finishes_first_is_returned() {
        let clock = FakeClockPort::default();
        let out = within(&clock, Duration::from_secs(5), async { 7 }).await;
        assert_eq!(out, Some(7));
    }

    #[tokio::test]
    async fn work_that_never_answers_is_cut_when_the_clock_passes_the_limit() {
        let clock = Arc::new(FakeClockPort::default());
        let waiting = {
            let clock = clock.clone();
            tokio::spawn(async move {
                within(
                    &*clock,
                    Duration::from_secs(5),
                    std::future::pending::<()>(),
                )
                .await
            })
        };
        while clock.pending_sleepers() == 0 {
            tokio::task::yield_now().await;
        }
        clock.advance(Duration::from_secs(5));
        assert_eq!(waiting.await.unwrap(), None);
    }
}
