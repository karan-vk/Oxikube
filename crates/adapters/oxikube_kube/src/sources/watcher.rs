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
//! Once the watches are registered the task re-reads the sources once, because a change made
//! between the first load and the registration produces no event.
//!
//! `Access` events are dropped: the reload itself reads the watched files and would otherwise
//! trigger itself on inotify.

use std::path::PathBuf;
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
///
/// A directory that cannot be watched is skipped and returned, so one bad path does not
/// disable the rest (the poll still covers it). Fails only when the watcher cannot start or
/// no directory could be watched.
fn start_watcher(
    dirs: &[PathBuf],
    tx: UnboundedSender<()>,
) -> OxiResult<(RecommendedWatcher, Vec<PathBuf>)> {
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
    let mut unwatched = Vec::new();
    for dir in dirs {
        if watcher.watch(dir, RecursiveMode::NonRecursive).is_err() {
            unwatched.push(dir.clone());
        }
    }
    if unwatched.len() == dirs.len() {
        return Err(OxiError::internal(
            "no kubeconfig directory could be watched; relying on the safety poll",
        ));
    }
    Ok((watcher, unwatched))
}

/// Registers the watches the current source list needs (blocking work, on the blocking pool).
async fn register(
    config: SourcesConfig,
    tx: UnboundedSender<()>,
) -> (Option<RecommendedWatcher>, WatchStatus, Vec<PathBuf>) {
    let setup = tokio::task::spawn_blocking(move || {
        let dirs = watch_dirs(&config);
        start_watcher(&dirs, tx)
    })
    .await;
    match setup {
        Ok(Ok((watcher, unwatched))) => (Some(watcher), WatchStatus::Active, unwatched),
        Ok(Err(err)) => (
            None,
            WatchStatus::Failed(err.message().to_owned()),
            Vec::new(),
        ),
        Err(err) => (None, WatchStatus::Failed(err.to_string()), Vec::new()),
    }
}

async fn run(inner: Weak<Inner>, config: SourcesConfig) {
    let (tx, mut rx) = unbounded_channel::<()>();
    // Keep the watcher alive for the life of the task. A failed setup leaves the poll running.
    let (mut _watcher, status, unwatched) = register(config.clone(), tx.clone()).await;
    let Some(strong) = inner.upgrade() else {
        return;
    };
    let mut rewatch = strong.rewatch.subscribe();
    // A change made after the first load read the files but before the watches existed has no
    // event; re-read once now (an empty diff when nothing changed). Done before publishing the
    // status, so `wait_for_watcher` returning means the catalog is current.
    let _ = strong.reload_if_loaded().await;
    strong.set_watch_status(status, unwatched);
    drop(strong);

    let mut poll = interval_at(Instant::now() + config.poll_interval, config.poll_interval);
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // `tx` stays alive here so `rx.recv()` pends instead of ending when the watcher is absent.
    loop {
        tokio::select! {
            _ = rx.recv() => settle(&mut rx, config.debounce).await,
            _ = poll.tick() => {}
            changed = rewatch.changed() => {
                if changed.is_err() {
                    return;
                }
                // The source list changed (and was reloaded by whoever changed it): watch the
                // directories of the new list. The old watcher is dropped after the new one
                // is registered, so no event is lost in between.
                let Some(strong) = inner.upgrade() else {
                    return;
                };
                let (next, status, unwatched) = register(strong.config(), tx.clone()).await;
                _watcher = next;
                strong.set_watch_status(status, unwatched);
                continue;
            }
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
