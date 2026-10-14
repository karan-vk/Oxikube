//! The task that reads one cluster's events feed into the service: opens the port's feed,
//! applies each batch, and opens it again with a doubling pause when it ends or cannot open.
//! A final failure stops it until a retry, a reconnect or a new namespace selection.

use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::OxiError;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::WatchScope;
use oxikube_ports::{ClockPort, EventsFeed, EventsPort, EventsWatchOptions};

use super::cluster::Inner;
use super::state::EventsState;

/// What one feed task needs.
pub(crate) struct FeedTask {
    pub(crate) inner: Weak<Inner>,
    pub(crate) cluster: ClusterId,
    pub(crate) generation: u64,
    pub(crate) port: Arc<dyn EventsPort>,
    pub(crate) options: EventsWatchOptions,
    pub(crate) clock: Arc<dyn ClockPort>,
    /// The first and the longest pause between two attempts.
    pub(crate) retry: (Duration, Duration),
}

/// How a feed (or an attempt to open one) ended.
enum Ended {
    /// The feed stopped (or did not open) for a reason that may pass: open it again.
    Retry(OxiError),
    /// A final failure, or the service is gone: stop.
    Stop,
}

/// Runs the feed of `task.cluster` until a final failure or the service is dropped. Aborted by
/// the guard the service keeps.
pub(crate) async fn run(task: FeedTask) {
    let mut delay = task.retry.0;
    loop {
        match read_once(&task, &mut delay).await {
            Ended::Stop => return,
            Ended::Retry(error) => {
                let Some(inner) = task.inner.upgrade() else {
                    return;
                };
                inner.set_state(
                    &task.cluster,
                    task.generation,
                    EventsState::retrying(&error),
                );
            }
        }
        task.clock.sleep(delay).await;
        delay = (delay * 2).min(task.retry.1);
    }
}

/// Opens the feed once and reads it until it ends.
async fn read_once(task: &FeedTask, delay: &mut Duration) -> Ended {
    let opened = task
        .port
        .watch_events(&WatchScope::Cluster, &task.options)
        .await;
    let EventsFeed { mut feed, monitor } = match opened {
        Ok(feed) => feed,
        Err(error) => return failed(task, error),
    };
    let apis = monitor.status().apis;
    match task.inner.upgrade() {
        Some(inner) if inner.set_monitor(&task.cluster, task.generation, monitor) => {}
        _ => return Ended::Stop,
    }
    tracing::debug!(cluster = %task.cluster, ?apis, capacity = ?task.options.capacity, "the events feed opened");
    while let Some(item) = feed.next().await {
        let Some(inner) = task.inner.upgrade() else {
            return Ended::Stop;
        };
        match item {
            Ok(batch) => {
                inner.apply_batch(&task.cluster, task.generation, batch);
                *delay = task.retry.0;
            }
            Err(error) if error.is_retryable() => {
                // The adapter backs off and relists on its own; the events held stay. Its next
                // batch (an empty one when the recovery changed nothing) says it reads again,
                // and `apply_batch` moves the state back to `Ready`.
                inner.set_state(
                    &task.cluster,
                    task.generation,
                    EventsState::retrying(&error),
                );
            }
            Err(error) => return failed(task, error),
        }
    }
    Ended::Retry(OxiError::network("the events feed ended"))
}

fn failed(task: &FeedTask, error: OxiError) -> Ended {
    if error.is_retryable() {
        return Ended::Retry(error);
    }
    tracing::warn!(cluster = %task.cluster, kind = ?error.kind(), "the events feed failed");
    if let Some(inner) = task.inner.upgrade() {
        inner.set_state(&task.cluster, task.generation, EventsState::failed(&error));
    }
    Ended::Stop
}
