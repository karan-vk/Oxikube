//! [`UserAliasesFile`]: the user's `aliases.json`, read once and reloaded when it changes.
//!
//! Like `settings.json` and `keymap.json` it is watched with [`SettingsFileWatcher`] (the
//! directory, debounced, atomic-save aware). Unlike them nothing in it needs GPUI: the watcher's
//! thread reads and parses the file and calls `on_change` with the result, so a reload costs the
//! UI thread nothing. A file that is not valid JSON keeps the aliases of the last good one and
//! reports why.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use oxikube_domain::{OxiError, OxiResult};

use crate::watcher::{DEFAULT_DEBOUNCE, SettingsFileWatcher};

use super::parse::{AliasDiagnostic, UserAlias, parse_aliases};

/// The aliases in effect and what was wrong with the file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadedAliases {
    /// The aliases in effect: the file's valid entries, or the last good ones when the file is
    /// not valid JSON.
    pub aliases: Vec<UserAlias>,
    /// Problems with the file and its entries, with lines.
    pub diagnostics: Vec<AliasDiagnostic>,
}

type OnChange = Arc<dyn Fn(&LoadedAliases) + Send + Sync>;

struct Shared {
    loaded: Mutex<LoadedAliases>,
    on_change: OnChange,
}

impl Shared {
    /// Applies new file text; calls `on_change` when the aliases or the problems differ from
    /// what they were.
    fn apply(&self, text: &str) {
        let next = match parse_aliases(text) {
            Ok(parsed) => LoadedAliases {
                aliases: parsed.aliases,
                diagnostics: parsed.diagnostics,
            },
            Err(problem) => {
                let mut kept = self.snapshot();
                kept.diagnostics = vec![problem];
                kept
            }
        };
        {
            let mut loaded = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
            if *loaded == next {
                return;
            }
            *loaded = next.clone();
        }
        (self.on_change)(&next);
    }

    fn snapshot(&self) -> LoadedAliases {
        self.loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The user's aliases file. Dropping it stops the watch.
pub struct UserAliasesFile {
    path: PathBuf,
    shared: Arc<Shared>,
    _watcher: Option<SettingsFileWatcher>,
}

impl std::fmt::Debug for UserAliasesFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserAliasesFile")
            .field("path", &self.path)
            .field("watching", &self._watcher.is_some())
            .finish_non_exhaustive()
    }
}

impl UserAliasesFile {
    /// Reads `path` (a missing file is no aliases), calls `on_change` once with the result and,
    /// when `watch` is set, again after every change of the file. `on_change` runs on the
    /// watcher's thread (or, for the first call and [`reload`](Self::reload), on the caller's),
    /// so it must be quick and thread-safe.
    ///
    /// Tests pass `watch: false` (GPUI's test scheduler forbids the watcher's OS thread) and call
    /// [`reload`](Self::reload) themselves.
    ///
    /// # Errors
    ///
    /// The file exists but cannot be read, or the watcher cannot start. The caller should log it:
    /// the aliases are then just the built-in ones.
    pub fn open(
        path: PathBuf,
        watch: bool,
        on_change: impl Fn(&LoadedAliases) + Send + Sync + 'static,
    ) -> OxiResult<Self> {
        let text = read_or_empty(&path)?;
        let shared = Arc::new(Shared {
            loaded: Mutex::new(LoadedAliases::default()),
            on_change: Arc::new(on_change),
        });
        shared.apply(&text);
        // The first call reports even an empty file, so the receiver starts from a known state.
        if shared.snapshot() == LoadedAliases::default() {
            (shared.on_change)(&LoadedAliases::default());
        }
        let watcher = if watch {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|err| {
                    OxiError::internal(format!("could not create {}", dir.display()))
                        .with_source(err)
                })?;
            }
            let target = shared.clone();
            Some(SettingsFileWatcher::spawn(
                path.clone(),
                text,
                DEFAULT_DEBOUNCE,
                move |text| {
                    target.apply(&text);
                    true
                },
            )?)
        } else {
            None
        };
        Ok(Self {
            path,
            shared,
            _watcher: watcher,
        })
    }

    /// Applies `text` as the file's new contents, as the watcher does.
    pub fn reload(&self, text: &str) {
        self.shared.apply(text);
    }

    /// Reads the file again and applies it.
    ///
    /// # Errors
    ///
    /// The file cannot be read.
    pub fn reload_from_disk(&self) -> OxiResult<()> {
        let text = read_or_empty(&self.path)?;
        self.reload(&text);
        Ok(())
    }

    /// The aliases in effect and the problems found by the last load.
    pub fn loaded(&self) -> LoadedAliases {
        self.shared.snapshot()
    }
}

/// The text of `path`; a missing file reads as empty text.
fn read_or_empty(path: &Path) -> OxiResult<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => {
            Err(OxiError::internal(format!("could not read {}", path.display())).with_source(err))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use oxikube_domain::AliasTarget;
    use oxikube_domain::ids::Gvr;

    use super::*;

    fn recorder() -> (
        impl Fn(&LoadedAliases) + Send + Sync + 'static,
        mpsc::Receiver<LoadedAliases>,
    ) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        (
            move |loaded: &LoadedAliases| {
                let _ = tx.lock().unwrap().send(loaded.clone());
            },
            rx,
        )
    }

    fn names(loaded: &LoadedAliases) -> Vec<&str> {
        loaded.aliases.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn a_missing_file_reports_no_aliases_once() {
        let dir = tempfile::tempdir().unwrap();
        let (on_change, rx) = recorder();
        let file =
            UserAliasesFile::open(dir.path().join("aliases.json"), false, on_change).unwrap();
        assert_eq!(rx.try_recv().unwrap(), LoadedAliases::default());
        assert!(rx.try_recv().is_err());
        assert_eq!(file.loaded(), LoadedAliases::default());
    }

    #[test]
    fn an_explicit_reload_applies_new_text_and_only_reports_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aliases.json");
        std::fs::write(&path, r#"{"a": "v1/pods"}"#).unwrap();
        let (on_change, rx) = recorder();
        let file = UserAliasesFile::open(path, false, on_change).unwrap();
        assert_eq!(names(&rx.try_recv().unwrap()), ["a"]);

        file.reload(r#"{"a": "v1/pods"}"#);
        assert!(rx.try_recv().is_err(), "same text, no report");

        file.reload(r#"{"a": "v1/pods", "b": "v1/nodes"}"#);
        assert_eq!(names(&rx.try_recv().unwrap()), ["a", "b"]);
        file.reload("{}");
        assert!(rx.try_recv().unwrap().aliases.is_empty());
    }

    #[test]
    fn a_broken_file_keeps_the_last_good_aliases_and_says_why() {
        let (on_change, rx) = recorder();
        let dir = tempfile::tempdir().unwrap();
        let file =
            UserAliasesFile::open(dir.path().join("aliases.json"), false, on_change).unwrap();
        rx.try_recv().unwrap();
        file.reload(r#"{"a": "v1/pods"}"#);
        rx.try_recv().unwrap();

        file.reload("{\n \"a\": \n");
        let after = rx.try_recv().unwrap();
        assert_eq!(names(&after), ["a"], "the last good aliases stay");
        assert_eq!(after.diagnostics.len(), 1);
        assert!(after.diagnostics[0].line.is_some());

        // Fixing the file clears the report.
        file.reload(r#"{"a": "v1/pods"}"#);
        let fixed = rx.try_recv().unwrap();
        assert!(fixed.diagnostics.is_empty());
    }

    #[test]
    fn bad_entries_are_reported_and_good_ones_load() {
        let dir = tempfile::tempdir().unwrap();
        let (on_change, rx) = recorder();
        let file =
            UserAliasesFile::open(dir.path().join("aliases.json"), false, on_change).unwrap();
        rx.try_recv().unwrap();
        file.reload("{\n \"ok\": \"v1/pods\",\n \"bad\": 3\n}");
        let loaded = rx.try_recv().unwrap();
        assert_eq!(names(&loaded), ["ok"]);
        assert_eq!(loaded.diagnostics[0].line, Some(3));
        assert_eq!(
            loaded.aliases[0].target,
            AliasTarget::Gvr(Gvr::new("", "v1", "pods"))
        );
    }

    #[test]
    fn an_unreadable_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be.
        let (on_change, _rx) = recorder();
        assert!(UserAliasesFile::open(dir.path().to_path_buf(), false, on_change).is_err());
    }

    /// Real file, real watcher: a save arrives as new aliases.
    #[test]
    fn a_saved_edit_is_reloaded_by_the_watcher() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aliases.json");
        std::fs::write(&path, "{}").unwrap();
        let (on_change, rx) = recorder();
        let _file = UserAliasesFile::open(path.clone(), true, on_change).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();

        // The platform watch starts asynchronously: rewrite until the change lands.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut seen = None;
        while seen.is_none() && Instant::now() < deadline {
            std::fs::write(&path, r#"{"mine": "v1/pods"}"#).unwrap();
            seen = rx.recv_timeout(Duration::from_millis(500)).ok();
        }
        assert_eq!(names(&seen.expect("no reload within 20 s")), ["mine"]);
    }
}
