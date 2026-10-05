//! Hot reload of the `themes/` directory: a `notify` watcher plus a thread that rescans.
//!
//! Independent of GPUI. The thread does all the file I/O and JSON parsing, so the UI thread only
//! swaps the finished result into the registry. Details that matter:
//!
//! * **The directory is watched** (non-recursively), so files created, renamed over or deleted
//!   are all seen; editors save by rename.
//! * **Bursts are debounced**: one save is several events, so the thread waits for a quiet
//!   period (at most ten periods, so a chatty directory cannot starve reloads), then scans once.
//! * **The first scan happens on the thread**, right after the watch is registered, so startup
//!   never reads the directory on the UI thread and a file written between "scan" and "watch"
//!   cannot be missed.
//! * **A scan equal to the previous one is not reported**, so touching a file without changing
//!   it does not re-theme the window.
//! * **Owned, not detached**: dropping [`ThemeDirWatcher`] closes the event channel and ends
//!   the thread; so does the callback returning `false`.
//!
//! Tests that run under GPUI's deterministic scheduler must not start this (it is an OS
//! thread) unless they allow parking (see `tests/watch_gpui.rs`).

use crate::user_dir::{UserThemes, scan_dir};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Quiet period before a rescan.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(100);

/// Why a watcher could not start.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    /// The directory could not be created or watched.
    #[error("cannot watch {path}: {message}")]
    Watch {
        /// The themes directory.
        path: PathBuf,
        /// The underlying error.
        message: String,
    },
}

/// Watches one themes directory and reports each changed scan.
pub struct ThemeDirWatcher {
    // Field order matters: the watcher drops first, which ends the thread.
    _watcher: RecommendedWatcher,
    _thread: JoinHandle<()>,
}

impl std::fmt::Debug for ThemeDirWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThemeDirWatcher").finish_non_exhaustive()
    }
}

impl ThemeDirWatcher {
    /// Starts watching `dir`, creating it when missing so a theme dropped in later is seen.
    ///
    /// `on_scan` runs on the watcher thread with the first scan and then each changed one;
    /// return `false` to stop.
    pub fn spawn(
        dir: PathBuf,
        debounce: Duration,
        on_scan: impl FnMut(UserThemes) -> bool + Send + 'static,
    ) -> Result<Self, WatchError> {
        let watch_error = |message: String| WatchError::Watch {
            path: dir.clone(),
            message,
        };
        std::fs::create_dir_all(&dir).map_err(|err| watch_error(err.to_string()))?;

        let (tx, rx) = channel::<()>();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            // Runs on notify's thread: filter and forward, nothing else. An error (overflow,
            // dropped watch) may hide a change, so it also triggers a rescan.
            let relevant = match &event {
                Ok(event) => !matches!(event.kind, EventKind::Access(_)),
                Err(_) => true,
            };
            if relevant {
                let _ = tx.send(());
            }
        })
        .map_err(|err| watch_error(err.to_string()))?;
        watcher
            .watch(&dir, RecursiveMode::NonRecursive)
            .map_err(|err| watch_error(err.to_string()))?;

        let scan_dir = dir.clone();
        let thread = std::thread::Builder::new()
            .name("oxikube-theme-watch".into())
            .spawn(move || run(&scan_dir, debounce, &rx, on_scan))
            .map_err(|err| watch_error(err.to_string()))?;
        Ok(Self {
            _watcher: watcher,
            _thread: thread,
        })
    }
}

fn run(
    dir: &std::path::Path,
    debounce: Duration,
    rx: &Receiver<()>,
    mut on_scan: impl FnMut(UserThemes) -> bool,
) {
    let mut last: Option<UserThemes> = None;
    let mut pending = true;
    loop {
        if !pending {
            if rx.recv().is_err() || !settle(rx, debounce) {
                return;
            }
        }
        pending = false;
        let scan = scan_dir(dir);
        if last.as_ref() != Some(&scan) {
            last = Some(scan.clone());
            if !on_scan(scan) {
                return;
            }
        }
    }
}

/// Waits for `debounce` without events, up to ten periods. `false` when the channel closed.
fn settle(rx: &Receiver<()>, debounce: Duration) -> bool {
    let deadline = Instant::now() + debounce * 10;
    while Instant::now() < deadline {
        match rx.recv_timeout(debounce) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => return true,
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::AYU;
    use std::sync::mpsc;

    fn wait_for(
        rx: &mpsc::Receiver<UserThemes>,
        timeout: Duration,
        pred: impl Fn(&UserThemes) -> bool,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(scan) if pred(&scan) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        false
    }

    /// Real directory, real watcher, no GPUI: the first scan arrives, a dropped-in file shows
    /// up, and deleting it removes it again.
    #[test]
    fn reports_files_dropped_into_and_removed_from_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let themes = dir.path().join("themes");
        let (tx, rx) = mpsc::channel();
        let _watcher = ThemeDirWatcher::spawn(themes.clone(), DEFAULT_DEBOUNCE, move |scan| {
            tx.send(scan).is_ok()
        })
        .unwrap();
        assert!(themes.is_dir(), "the directory is created");
        assert!(wait_for(&rx, Duration::from_secs(10), |scan| scan
            .families
            .is_empty()));

        let file = themes.join("ayu.json");
        // The platform watch starts asynchronously (FSEvents): rewrite until it lands.
        let landed = (0..20).any(|_| {
            std::fs::write(&file, AYU).unwrap();
            wait_for(&rx, Duration::from_millis(500), |scan| {
                scan.families.len() == 1
            })
        });
        assert!(landed, "no scan with the new file within 10 s");

        std::fs::remove_file(&file).unwrap();
        assert!(wait_for(&rx, Duration::from_secs(10), |scan| scan
            .families
            .is_empty()));
    }

    #[test]
    fn dropping_the_watcher_ends_the_thread() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel::<UserThemes>();
        let watcher =
            ThemeDirWatcher::spawn(dir.path().join("themes"), DEFAULT_DEBOUNCE, move |scan| {
                tx.send(scan).is_ok()
            })
            .unwrap();
        drop(watcher);
        // The thread owns the only sender: once it exits the channel disconnects.
        let outcome = loop {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(_) => {}
                Err(err) => break err,
            }
        };
        assert_eq!(outcome, mpsc::RecvTimeoutError::Disconnected);
    }
}
