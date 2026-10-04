//! GPUI wiring: install the [`SettingsStore`] global, load the user file, hot reload, and
//! write typed edits back to `settings.json`.
//!
//! Startup reads the (small, local) user file synchronously so the first frame already has
//! the user's settings; that read plus parsing stays well under the 30 ms main-thread budget
//! (E05-S13). After that, the watcher thread does all file reads and the background executor
//! does all writes; the UI thread only parses and re-resolves.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Global, Task, UpdateGlobal as _};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};

use crate::paths::{self, io_error};
use crate::settings::Settings;
use crate::store::SettingsStore;
use crate::update::new_text_for_update;
use crate::watcher::{DEFAULT_DEBOUNCE, SettingsFileWatcher};

impl Global for SettingsStore {}

/// Serialises read-modify-write cycles on the user file across concurrent edits.
static FILE_EDIT_LOCK: Mutex<()> = Mutex::new(());
/// Orders completed writes so a late continuation never applies an older text.
static WRITE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Keeps the hot-reload watcher and its apply task alive for the life of the app.
struct SettingsWatch {
    _watcher: SettingsFileWatcher,
    _apply: Task<()>,
}

impl Global for SettingsWatch {}

/// Install the settings store from the default config dir (see [`paths::config_dir`]),
/// creating `settings.json` on first run, and start hot reload.
///
/// Problems (no home dir, unreadable file, invalid JSON) are logged and leave the defaults or
/// last good settings in effect; they never stop startup.
pub fn init(cx: &mut App) {
    match paths::config_dir() {
        Some(dir) => install(cx, Some(&dir), true),
        None => {
            tracing::warn!("no config directory; using default settings only");
            install(cx, None, false);
        }
    }
}

/// Install the settings store reading `<config_dir>/settings.json`, without a file watcher.
///
/// For tests and tools: GPUI's deterministic test scheduler forbids the watcher's OS thread.
/// Edits through [`update_user_settings`] still write the file and apply immediately.
pub fn init_with_dir(config_dir: &Path, cx: &mut App) {
    install(cx, Some(config_dir), false);
}

fn install(cx: &mut App, config_dir: Option<&Path>, watch: bool) {
    let mut store = match SettingsStore::new(oxikube_assets::default_settings()) {
        Ok(store) => store,
        Err(err) => {
            tracing::error!(%err, "embedded default settings are invalid");
            SettingsStore::empty()
        }
    };
    let Some(path) = config_dir.map(paths::user_settings_path) else {
        cx.set_global(store);
        return;
    };

    let text = match paths::load_or_create_user_settings(&path) {
        Ok(text) => Some(text),
        Err(err) => {
            tracing::warn!(%err, "could not load user settings; using defaults");
            None
        }
    };
    if let Some(text) = &text {
        apply_user_text(&mut store, text);
    }
    store.set_user_settings_path(Some(path.clone()));
    cx.set_global(store);

    if watch && let Some(text) = text {
        start_watch(cx, path, text);
    }
}

fn apply_user_text(store: &mut SettingsStore, text: &str) {
    if let Err(err) = store.set_user_settings(text) {
        tracing::warn!(%err, "keeping the last good settings");
    }
    for diagnostic in store.diagnostics() {
        tracing::warn!(%diagnostic, "settings");
    }
}

fn start_watch(cx: &mut App, path: PathBuf, initial_text: String) {
    let (tx, mut rx) = mpsc::unbounded::<String>();
    let watcher =
        match SettingsFileWatcher::spawn(path, initial_text, DEFAULT_DEBOUNCE, move |text| {
            tx.unbounded_send(text).is_ok()
        }) {
            Ok(watcher) => watcher,
            Err(err) => {
                tracing::warn!(%err, "settings hot reload is off");
                return;
            }
        };
    let apply = cx.spawn(async move |cx| {
        while let Some(text) = rx.next().await {
            cx.update_global::<SettingsStore, _>(|store, _| apply_user_text(store, &text));
        }
    });
    cx.set_global(SettingsWatch {
        _watcher: watcher,
        _apply: apply,
    });
}

/// Edit `T`'s content in the user's `settings.json`, keeping comments and formatting, and
/// apply the result immediately (before the watcher sees the write).
///
/// `cluster: None` edits the root of the file; `Some(id)` edits `clusters.<id>`. File I/O
/// runs on the background executor. A store without a file (no config dir) edits its
/// in-memory text only.
pub fn update_user_settings<T: Settings>(
    cx: &mut App,
    cluster: Option<ClusterId>,
    update: impl FnOnce(&mut T::Content) + Send + 'static,
) -> Task<OxiResult<()>> {
    let store = cx.global::<SettingsStore>();
    let Some(path) = store.user_settings_path().map(Path::to_path_buf) else {
        let old_text = store.user_settings_text().unwrap_or_default().to_owned();
        let result = new_text_for_update::<T>(&old_text, cluster.as_ref(), update)
            .and_then(|text| SettingsStore::update_global(cx, |s, _| s.set_user_settings(&text)));
        return Task::ready(result);
    };

    let write = cx.background_spawn(async move {
        // A poisoned lock only means another edit panicked; the file is still consistent.
        let _guard = FILE_EDIT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let old_text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(io_error("read", &path, err)),
        };
        let new_text = new_text_for_update::<T>(&old_text, cluster.as_ref(), update)?;
        if new_text != old_text {
            std::fs::write(&path, &new_text).map_err(|err| io_error("write", &path, err))?;
        }
        let seq = WRITE_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
        Ok::<_, OxiError>((seq, new_text))
    });
    cx.spawn(async move |cx| {
        let (seq, new_text) = write.await?;
        cx.update_global::<SettingsStore, _>(|store, _| store.apply_written_text(seq, &new_text))
    })
}
