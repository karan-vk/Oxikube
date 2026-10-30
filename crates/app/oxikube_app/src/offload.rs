//! [`off_io_workers`]: CPU-heavy work moved off the Tokio runtime's I/O workers.
//!
//! The Kubernetes runtime has two worker threads (`oxikube_runtime`'s `DEFAULT_WORKER_THREADS`)
//! that drive every watch stream, port-forward, exec session and request. A comparison of two
//! multi-megabyte objects or the parse of a manifest directory of tens of megabytes holds one of
//! them for hundreds of milliseconds, and the watch deltas, terminal bytes and tunnels queued on it
//! wait that long. Such work goes to the runtime's blocking pool instead, as the Helm manifest
//! parse does: the worker stays free while it runs.

use oxikube_domain::{OxiError, OxiResult};

/// Runs `work` on the Tokio runtime's blocking pool and waits for it without holding a worker;
/// with no runtime (a test polling with `now_or_never`, a plain executor) it runs in place.
///
/// The work is not cancelled when the returned future is dropped: give it a bound of its own
/// (the line diff's timeout, the load limits).
///
/// # Errors
///
/// `Internal` when the work panicked.
pub async fn off_io_workers<T, F>(work: F) -> OxiResult<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => runtime
            .spawn_blocking(work)
            .await
            .map_err(|error| OxiError::internal(format!("background work stopped: {error}"))),
        Err(_) => Ok(work()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use futures::FutureExt as _;

    use super::off_io_workers;

    /// On a runtime with one thread, work that waits for another task of the same runtime only
    /// finishes when it does not hold that thread.
    #[test]
    fn the_work_leaves_the_runtime_worker_free() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime");
        let (sender, receiver) = mpsc::channel::<()>();
        let finished = runtime.block_on(async move {
            let other = tokio::spawn(async move {
                // Yields first, so the offloaded work starts before this sends.
                tokio::task::yield_now().await;
                sender.send(()).expect("the work is waiting");
            });
            let waited =
                off_io_workers(move || receiver.recv_timeout(Duration::from_secs(5)).is_ok())
                    .await
                    .expect("the work ran");
            other.await.expect("the other task ran");
            waited
        });
        assert!(
            finished,
            "the other task ran while the work was in progress"
        );
    }

    #[test]
    fn without_a_runtime_the_work_runs_in_place() {
        let out = off_io_workers(|| 7)
            .now_or_never()
            .expect("no runtime: done at once");
        assert_eq!(out.expect("ran"), 7);
    }

    #[test]
    fn a_panic_is_an_error() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime");
        let out = runtime.block_on(off_io_workers(|| -> u8 { panic!("boom") }));
        assert!(out.is_err());
    }
}
