//! Hot reload: a `notify` watcher on the settings file's directory with a debounce thread.
//!
//! Independent of GPUI so it can be tested with a real file. Details that matter:
//!
//! * **The directory is watched, not the file.** Editors save by renaming a new file over
//!   the old one; a watch on the old inode would go silent.
//! * **Bursts are debounced.** One save produces several events; the thread waits for a
//!   quiet period (capped at ten periods so a chatty directory cannot starve reloads), then
//!   reads the file once and calls back only if the text changed.
//! * **Read failures can be reported.** [`SettingsFileWatcher::spawn_reporting`] also calls
//!   back with the error of a failed read (invalid UTF-8, permission denied, a directory), once
//!   per distinct error, and again with the text when the file reads again, even if that text is
//!   the one from before the failure, so the caller can clear what it showed.
//! * **The thread does the I/O.** The callback receives the new text, so the UI thread never
//!   touches the disk on reload. A missing file reads as empty text (no overrides).
//! * **Owned, not detached.** Dropping [`SettingsFileWatcher`] drops the `notify` watcher,
//!   which closes the event channel and ends the thread. The callback returning `false`
//!   (its receiver is gone) also ends it.
//!
//! Tests that run under GPUI's deterministic scheduler must not start this (it is an OS
//! thread); use [`crate::init_with_dir`], which has no watcher.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use oxikube_domain::{OxiError, OxiResult};

use crate::paths;

/// Quiet period before a reload; keeps the save-to-applied latency well under one second.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(100);

/// Watches one settings file and reports its new text after each settled change.
pub struct SettingsFileWatcher {
    // Field order matters: the watcher drops first, which ends the thread.
    _watcher: RecommendedWatcher,
    _thread: JoinHandle<()>,
}

impl std::fmt::Debug for SettingsFileWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsFileWatcher")
            .finish_non_exhaustive()
    }
}

impl SettingsFileWatcher {
    /// Start watching `path` (its directory must exist). `on_change` runs on the watcher
    /// thread with the file's new text; return `false` to stop watching.
    ///
    /// `initial_text` is what the caller already loaded: a change made between that read and
    /// the watch registration is caught by one re-read right after the watch starts.
    pub fn spawn(
        path: PathBuf,
        initial_text: String,
        debounce: Duration,
        mut on_change: impl FnMut(String) -> bool + Send + 'static,
    ) -> OxiResult<Self> {
        Self::spawn_reporting(path, initial_text, debounce, move |read| match read {
            Ok(text) => on_change(text),
            Err(err) => {
                tracing::warn!(%err, "could not read settings file");
                true
            }
        })
    }

    /// [`Self::spawn`] for callers that surface a file they cannot read: `on_change` gets
    /// `Err` when a read fails (once per distinct error, not on every event) and `Ok` with the
    /// text when the file changed or reads again after a failure.
    pub fn spawn_reporting(
        path: PathBuf,
        initial_text: String,
        debounce: Duration,
        on_change: impl FnMut(OxiResult<String>) -> bool + Send + 'static,
    ) -> OxiResult<Self> {
        // The link's own name in its directory, plus the resolved file in its directory
        // when the settings file is a symlink.
        let mut watched = vec![dir_and_name(&path)?];
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_symlink()) {
            match std::fs::canonicalize(&path) {
                Ok(target) => {
                    let target = dir_and_name(&target)?;
                    if !watched.contains(&target) {
                        watched.push(target);
                    }
                }
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "settings symlink is dangling");
                }
            }
        }
        let file_names: Vec<OsString> = watched.iter().map(|(_, name)| name.clone()).collect();

        let (tx, rx) = channel::<()>();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            // Runs on notify's thread: filter and forward, nothing else.
            let relevant = match &event {
                Ok(event) => {
                    !matches!(event.kind, EventKind::Access(_))
                        && (event.paths.is_empty()
                            || event.paths.iter().any(|p| {
                                p.file_name()
                                    .is_some_and(|name| file_names.iter().any(|n| n == name))
                            }))
                }
                // An error (overflow, dropped watch) may hide a change: re-read.
                Err(_) => true,
            };
            if relevant {
                let _ = tx.send(());
            }
        })
        .map_err(|err| {
            OxiError::internal("could not start the settings watcher").with_source(err)
        })?;
        let dirs: BTreeSet<&PathBuf> = watched.iter().map(|(dir, _)| dir).collect();
        for dir in dirs {
            watcher
                .watch(dir, RecursiveMode::NonRecursive)
                .map_err(|err| {
                    OxiError::internal(format!("could not watch {}", dir.display()))
                        .with_source(err)
                })?;
        }

        let thread = std::thread::Builder::new()
            .name("oxikube-settings-watch".into())
            .spawn(move || run(&path, initial_text, debounce, &rx, on_change))
            .map_err(|err| {
                OxiError::internal("could not start the settings watch thread").with_source(err)
            })?;

        Ok(Self {
            _watcher: watcher,
            _thread: thread,
        })
    }
}

/// The directory to watch for `path` and the file name to filter its events on.
fn dir_and_name(path: &Path) -> OxiResult<(PathBuf, OsString)> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let name = path
        .file_name()
        .ok_or_else(|| OxiError::internal("settings path has no file name"))?
        .to_owned();
    Ok((dir, name))
}

fn run(
    path: &Path,
    mut last_text: String,
    debounce: Duration,
    rx: &Receiver<()>,
    mut on_change: impl FnMut(OxiResult<String>) -> bool,
) {
    // The message of the failure last reported and not yet followed by a good read.
    let mut failed: Option<String> = None;
    // Catch a change made before the watch was registered.
    let mut pending = true;
    loop {
        if !pending {
            if rx.recv().is_err() {
                return;
            }
            if !settle(rx, debounce) {
                return;
            }
        }
        pending = false;
        let text = match paths::read_or_empty(path) {
            Ok(text) => text,
            Err(err) => {
                let message = err.to_string();
                if failed.as_deref() != Some(&message) {
                    failed = Some(message);
                    let err = OxiError::internal(format!("could not read {}", path.display()))
                        .with_source(err);
                    if !on_change(Err(err)) {
                        return;
                    }
                }
                continue;
            }
        };
        // After a failure the text is reported even when unchanged: the caller is showing the
        // failure and must be told that it is over.
        if text != last_text || failed.take().is_some() {
            last_text.clone_from(&text);
            if !on_change(Ok(text)) {
                return;
            }
        }
    }
}

/// Wait for `debounce` without events, up to ten periods. `false` when the channel closed.
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
    use std::sync::mpsc;

    /// Receive until `expected` arrives or `timeout` passes.
    fn wait_for(rx: &mpsc::Receiver<String>, expected: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(text) if text == expected => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        false
    }

    /// Real file, real watcher, no GPUI: an edit arrives as new text within the hot-reload
    /// budget, and an unchanged rewrite does not.
    #[test]
    fn reports_edits_to_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{}").unwrap();

        let (tx, rx) = mpsc::channel();
        let _watcher = SettingsFileWatcher::spawn(path.clone(), "{}".into(), DEFAULT_DEBOUNCE, {
            move |text| tx.send(text).is_ok()
        })
        .unwrap();

        // The platform watch starts asynchronously (FSEvents), and the watcher's start-up
        // re-read may catch the file mid-write: rewrite until the final text lands instead of
        // sleeping a guessed amount.
        let landed = (0..20).any(|_| {
            std::fs::write(&path, "{\"a\": 1}").unwrap();
            wait_for(&rx, "{\"a\": 1}", Duration::from_millis(500))
        });
        assert!(landed, "no reload within 10 s");

        // Atomic save: write a temp file and rename it over the watched one.
        let tmp = dir.path().join("settings.json.tmp");
        std::fs::write(&tmp, "{\"a\": 2}").unwrap();
        let saved = Instant::now();
        std::fs::rename(&tmp, &path).unwrap();
        assert!(wait_for(&rx, "{\"a\": 2}", Duration::from_secs(10)));
        // Budget: applied within 1 s of the save. Allow slack for loaded CI machines.
        let latency = saved.elapsed();
        assert!(latency < Duration::from_secs(2), "reload took {latency:?}");

        // Same content again: no callback.
        std::fs::write(&path, "{\"a\": 2}").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(600)).is_err());
    }

    /// `settings.json` symlinked into another directory (dotfile managers): an edit of the
    /// real file is reported although the link's directory sees no event.
    #[test]
    #[cfg(unix)]
    fn reports_edits_through_a_symlink_into_another_directory() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let dotfiles = dir.path().join("dotfiles");
        std::fs::create_dir(&config).unwrap();
        std::fs::create_dir(&dotfiles).unwrap();
        let real = dotfiles.join("oxikube-settings.json");
        std::fs::write(&real, "{}").unwrap();
        let link = config.join("settings.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let (tx, rx) = mpsc::channel();
        let _watcher = SettingsFileWatcher::spawn(link, "{}".into(), DEFAULT_DEBOUNCE, {
            move |text| tx.send(text).is_ok()
        })
        .unwrap();

        // Rewrite until the watch is live (see `reports_edits_to_a_real_file`).
        let landed = (0..20).any(|_| {
            std::fs::write(&real, "{\"a\": 1}").unwrap();
            wait_for(&rx, "{\"a\": 1}", Duration::from_millis(500))
        });
        assert!(landed, "no reload within 10 s");

        // An editor's atomic save of the real file.
        let tmp = dotfiles.join("oxikube-settings.json.tmp");
        std::fs::write(&tmp, "{\"a\": 2}").unwrap();
        std::fs::rename(&tmp, &real).unwrap();
        assert!(wait_for(&rx, "{\"a\": 2}", Duration::from_secs(10)));
    }

    /// A read that fails is reported once, not on every event, and the same text that was there
    /// before the failure is reported again when the file reads fine, so a caller showing the
    /// failure can clear it.
    #[test]
    fn reports_a_failed_read_once_and_the_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keymap.json");
        std::fs::write(&path, "{}").unwrap();
        let (tx, rx) = mpsc::channel::<Result<String, String>>();
        let _watcher = SettingsFileWatcher::spawn_reporting(
            path.clone(),
            "{}".into(),
            DEFAULT_DEBOUNCE,
            move |read| tx.send(read.map_err(|err| err.to_string())).is_ok(),
        )
        .unwrap();

        let wait = |want_err: bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                match rx.recv_timeout(left) {
                    Ok(read) if read.is_err() == want_err => return Some(read),
                    Ok(_) => {}
                    Err(_) => return None,
                }
            }
            None
        };
        // Invalid UTF-8; rewrite until the platform watch is live.
        let failed = (0..20).find_map(|_| {
            std::fs::write(&path, [0xFF, 0xFE, b'{', 0]).unwrap();
            let deadline = Instant::now() + Duration::from_millis(500);
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                if matches!(rx.recv_timeout(left), Ok(Err(_))) {
                    return Some(());
                }
            }
            None
        });
        assert!(failed.is_some(), "the failed read was not reported");
        // The same failure again (another event for the same bytes) is not reported twice.
        std::fs::write(&path, [0xFF, 0xFE, b'{', 0]).unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(600)).is_err(),
            "one report per failure"
        );
        // Back to the text from before the failure: reported although unchanged.
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(wait(false), Some(Ok("{}".to_owned())));
    }

    #[test]
    fn dropping_the_watcher_ends_the_thread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        let (tx, rx) = mpsc::channel::<String>();
        let watcher =
            SettingsFileWatcher::spawn(path, "{}".into(), DEFAULT_DEBOUNCE, move |text| {
                tx.send(text).is_ok()
            })
            .unwrap();
        drop(watcher);
        // The thread owned the only sender; once it exits the channel disconnects.
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }
}
