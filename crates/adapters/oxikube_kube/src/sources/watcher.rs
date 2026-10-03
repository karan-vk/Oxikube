//! Change detection: a `notify` watcher plus a safety poll, both ending in one `reload()`.
//!
//! The pattern is kdash's `start_kubeconfig_watcher` (event watcher plus a periodic re-read);
//! the code is ours. Three details matter:
//!
//! * **Parent directories are watched, not files.** Editors and `kubectl config` replace a file
//!   by renaming a new one over it; a watch on the old inode would go silent.
//! * **The watcher thread never touches async tasks.** `notify` calls the handler on its own
//!   thread; the handler only pushes `()` into an unbounded tokio channel. Everything else runs
//!   in one adapter-owned tokio task.
//! * **Bursts are debounced.** A rename-over-write produces several events; the task waits for
//!   a quiet period (capped, so a chatty directory cannot starve reloads) and reloads once.
//!
//! `Access` events are dropped: the reload itself reads the watched files and would otherwise
//! trigger itself on inotify.

use std::sync::Weak;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use oxikube_domain::{OxiError, OxiResult};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval_at, timeout};

use super::layout::watch_dirs;
use super::{Inner, SourcesConfig, WatchStatus};

/// The watch task. Dropping it aborts the task, which drops the `notify` watcher and its thread.
pub(super) struct WatchGuard(JoinHandle<()>);

impl Drop for WatchGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Start the watch task. Needs a tokio runtime; fails when there is none.
///
/// The task holds only a [`Weak`] reference, so it also ends when the adapter is dropped.
pub(super) fn spawn(inner: Weak<Inner>, config: &SourcesConfig) -> OxiResult<WatchGuard> {
    let runtime = tokio::runtime::Handle::try_current().map_err(|err| {
        OxiError::internal("watching kubeconfig sources needs a tokio runtime").with_source(err)
    })?;
    let task = run(inner, config.clone());
    Ok(WatchGuard(runtime.spawn(task)))
}

/// Register a watch on each of `dirs`; the handler forwards relevant events to `tx`.
fn start_watcher(
    dirs: &[std::path::PathBuf],
    tx: UnboundedSender<()>,
) -> OxiResult<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        // Runs on notify's thread: only a channel send, no task wake-ups of our own.
        let relevant = match &event {
            Ok(event) => !matches!(event.kind, EventKind::Access(_)),
            // A watcher error (overflow, a dropped watch) may mean missed events: reload.
            Err(_) => true,
        };
        if relevant {
            let _ = tx.send(());
        }
    })
    .map_err(|err| OxiError::internal("could not start the kubeconfig watcher").with_source(err))?;
    for dir in dirs {
        watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|err| {
                OxiError::internal(format!("could not watch {}", dir.display())).with_source(err)
            })?;
    }
    Ok(watcher)
}

async fn run(inner: Weak<Inner>, config: SourcesConfig) {
    let (tx, mut rx) = unbounded_channel::<()>();
    let setup = {
        let tx = tx.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || {
            let dirs = watch_dirs(&config);
            start_watcher(&dirs, tx)
        })
        .await
    };
    // Keep the watcher alive for the life of the task. A failed setup leaves the poll running.
    let (_watcher, status) = match setup {
        Ok(Ok(watcher)) => (Some(watcher), WatchStatus::Active),
        Ok(Err(err)) => (None, WatchStatus::Failed(err.message().to_owned())),
        Err(err) => (None, WatchStatus::Failed(err.to_string())),
    };
    match inner.upgrade() {
        Some(inner) => inner.set_watch_status(status),
        None => return,
    }

    let mut poll = interval_at(Instant::now() + config.poll_interval, config.poll_interval);
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // `tx` stays alive here so `rx.recv()` pends instead of ending when the watcher is absent.
    let _keep_open = tx;
    loop {
        tokio::select! {
            _ = rx.recv() => settle(&mut rx, config.debounce).await,
            _ = poll.tick() => {}
        }
        let Some(inner) = inner.upgrade() else {
            return;
        };
        // The loader is tolerant; an error here is a failed blocking task. The next trigger retries.
        let _ = inner.reload().await;
    }
}

/// Wait for a quiet period of `debounce`, giving up after ten periods of continuous events.
async fn settle(rx: &mut tokio::sync::mpsc::UnboundedReceiver<()>, debounce: Duration) {
    let deadline = Instant::now() + debounce * 10;
    while Instant::now() < deadline && matches!(timeout(debounce, rx.recv()).await, Ok(Some(()))) {}
}
