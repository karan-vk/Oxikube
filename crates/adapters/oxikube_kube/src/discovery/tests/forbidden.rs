//! A refused CRD watch (#449): reported once, no hot retry loop, discovery keeps running on an
//! interval, and recovery is reported. Paused time, scripted watch connections.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;

use futures::StreamExt as _;
use futures::stream::{self, BoxStream};
use oxikube_ports::CrdWatchStatus;
use parking_lot::Mutex;
use tokio::time::sleep;

use crate::discovery::CrdWatchConfig;
use crate::discovery::crd_watch::{Signal, watch_loop};

const INTERVAL: Duration = Duration::from_secs(300);

fn config() -> CrdWatchConfig {
    CrdWatchConfig {
        debounce: Duration::from_millis(50),
        forbidden_interval: INTERVAL,
        ..CrdWatchConfig::default()
    }
}

fn refused() -> BoxStream<'static, Signal> {
    stream::iter([Signal::Forbidden("crds is forbidden".into())]).boxed()
}

fn running() -> BoxStream<'static, Signal> {
    stream::iter([Signal::Change])
        .chain(stream::pending())
        .boxed()
}

struct Run {
    attempts: Arc<AtomicU32>,
    refreshes: Arc<AtomicUsize>,
    statuses: Arc<Mutex<Vec<CrdWatchStatus>>>,
}

/// Starts the loop; connection `n` is `script(n)`.
fn start(script: impl Fn(u32) -> BoxStream<'static, Signal> + Send + 'static) -> Run {
    let run = Run {
        attempts: Arc::new(AtomicU32::new(0)),
        refreshes: Arc::new(AtomicUsize::new(0)),
        statuses: Arc::new(Mutex::new(Vec::new())),
    };
    let (attempts, refreshes, statuses) = (
        run.attempts.clone(),
        run.refreshes.clone(),
        run.statuses.clone(),
    );
    tokio::spawn(async move {
        let mut connection = 0;
        watch_loop(
            move || {
                connection += 1;
                script(connection)
            },
            config(),
            attempts,
            move || {
                refreshes.fetch_add(1, Ordering::SeqCst);
                async { true }
            },
            move |status| statuses.lock().push(status),
        )
        .await;
    });
    run
}

#[tokio::test(start_paused = true)]
async fn a_refused_watch_is_reported_and_retried_only_on_the_interval() {
    let run = start(|_| refused());
    sleep(Duration::from_secs(1)).await;
    assert_eq!(run.attempts.load(Ordering::SeqCst), 1, "no hot retry");
    assert!(
        matches!(run.statuses.lock().last(), Some(CrdWatchStatus::Forbidden { reason }) if reason.contains("forbidden")),
        "reported: {:?}",
        run.statuses.lock()
    );
    assert_eq!(
        run.refreshes.load(Ordering::SeqCst),
        1,
        "the fallback re-discovery ran"
    );

    sleep(Duration::from_secs(3600)).await;
    // One attempt per interval over the hour (plus the first), each with its fallback refresh.
    let attempts = run.attempts.load(Ordering::SeqCst);
    assert!(
        (12..=14).contains(&attempts),
        "attempts over an hour: {attempts}"
    );
    assert_eq!(run.refreshes.load(Ordering::SeqCst), attempts as usize);
}

#[tokio::test(start_paused = true)]
async fn the_watch_recovers_when_access_is_granted() {
    let run = start(|n| if n == 1 { refused() } else { running() });
    sleep(Duration::from_secs(1)).await;
    assert!(
        run.statuses
            .lock()
            .last()
            .is_some_and(CrdWatchStatus::is_forbidden)
    );

    sleep(INTERVAL).await;
    assert_eq!(run.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(run.statuses.lock().last(), Some(&CrdWatchStatus::Watching));
    let refreshes = run.refreshes.load(Ordering::SeqCst);
    sleep(Duration::from_secs(3600)).await;
    assert_eq!(
        run.attempts.load(Ordering::SeqCst),
        2,
        "a healthy watch is not restarted"
    );
    assert_eq!(
        run.refreshes.load(Ordering::SeqCst),
        refreshes,
        "and does not poll"
    );
}

#[tokio::test(start_paused = true)]
async fn a_healthy_watch_reports_watching_and_refreshes_on_signals() {
    let run = start(|_| running());
    sleep(Duration::from_secs(1)).await;
    assert_eq!(run.statuses.lock().last(), Some(&CrdWatchStatus::Watching));
    assert_eq!(run.refreshes.load(Ordering::SeqCst), 1);
}
