use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::channel::mpsc::{UnboundedSender, unbounded};
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::discovery::{CrdWatchConfig, crd_watch::debounce_refresh};

const MS: fn(u64) -> Duration = Duration::from_millis;

fn config() -> CrdWatchConfig {
    CrdWatchConfig {
        debounce: MS(500),
        max_wait: MS(3000),
        retry_delay: MS(5000),
    }
}

/// Starts the loop with a refresh that fails `failures` times, then succeeds. Returns the signal
/// sender, the refresh counter and the task.
fn start(failures: usize) -> (UnboundedSender<()>, Arc<AtomicUsize>, JoinHandle<()>) {
    let (tx, rx) = unbounded();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let task = tokio::spawn(debounce_refresh(rx, config(), move || {
        let attempt = counter.fetch_add(1, Ordering::SeqCst);
        async move { attempt >= failures }
    }));
    (tx, calls, task)
}

fn count(calls: &AtomicUsize) -> usize {
    calls.load(Ordering::SeqCst)
}

#[tokio::test(start_paused = true)]
async fn idle_watcher_never_refreshes() {
    let (_tx, calls, _task) = start(0);
    sleep(Duration::from_secs(3600)).await;
    assert_eq!(count(&calls), 0);
}

#[tokio::test(start_paused = true)]
async fn a_burst_collapses_into_one_refresh_after_the_quiet_period() {
    let (tx, calls, _task) = start(0);
    for _ in 0..50 {
        tx.unbounded_send(()).expect("send");
    }
    sleep(MS(400)).await;
    assert_eq!(count(&calls), 0, "still inside the debounce window");
    sleep(MS(200)).await;
    assert_eq!(count(&calls), 1);
    sleep(Duration::from_secs(60)).await;
    assert_eq!(count(&calls), 1, "nothing more without new signals");
}

#[tokio::test(start_paused = true)]
async fn each_signal_extends_the_quiet_period() {
    let (tx, calls, _task) = start(0);
    for _ in 0..4 {
        tx.unbounded_send(()).expect("send");
        sleep(MS(400)).await;
    }
    assert_eq!(
        count(&calls),
        0,
        "signals every 400 ms keep deferring a 500 ms debounce"
    );
    sleep(MS(200)).await;
    assert_eq!(count(&calls), 1);
}

#[tokio::test(start_paused = true)]
async fn a_continuous_burst_is_capped_by_max_wait() {
    let (tx, calls, _task) = start(0);
    for _ in 0..8 {
        tx.unbounded_send(()).expect("send");
        sleep(MS(400)).await;
    }
    // 3.2 s of signals every 400 ms: the 3 s cap fired once in the middle of the stream.
    assert_eq!(count(&calls), 1);
}

#[tokio::test(start_paused = true)]
async fn separate_bursts_refresh_separately() {
    let (tx, calls, _task) = start(0);
    tx.unbounded_send(()).expect("send");
    sleep(Duration::from_secs(1)).await;
    tx.unbounded_send(()).expect("send");
    sleep(Duration::from_secs(1)).await;
    assert_eq!(count(&calls), 2);
}

#[tokio::test(start_paused = true)]
async fn a_failed_refresh_is_retried_after_the_retry_delay() {
    let (tx, calls, _task) = start(2);
    tx.unbounded_send(()).expect("send");
    sleep(MS(600)).await;
    assert_eq!(count(&calls), 1, "first attempt failed");
    sleep(MS(4000)).await;
    assert_eq!(count(&calls), 1, "retry waits the full delay");
    sleep(MS(1500)).await;
    assert_eq!(count(&calls), 2, "second attempt failed");
    sleep(MS(5000)).await;
    assert_eq!(count(&calls), 3, "third attempt succeeded");
    sleep(Duration::from_secs(60)).await;
    assert_eq!(count(&calls), 3, "no retries after success");
}

#[tokio::test(start_paused = true)]
async fn the_loop_ends_when_the_signal_stream_ends() {
    let (tx, calls, task) = start(0);
    drop(tx);
    task.await.expect("task");
    assert_eq!(count(&calls), 0);
}
