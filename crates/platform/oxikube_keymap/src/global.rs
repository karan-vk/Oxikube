//! GPUI wiring: install the [`KeymapStore`] global, bind the merged layers, and hot reload
//! `keymap.json`.
//!
//! Start-up reads the (small, local) user file synchronously so the first frame already has the
//! user's bindings. After that the watcher thread reads the file and the UI thread only parses
//! and rebinds, which costs well under a millisecond per section (see the `merge_cost` test).
//! Keystroke dispatch itself is GPUI's and allocates nothing here.
//!
//! # Init order
//!
//! Call [`init`] after the settings store and before the crates that handle actions. Crates
//! that bind keys themselves with `cx.bind_keys` (the component library does) keep their
//! bindings across reloads, and the keymap layers are always re-added after them, so the user's
//! file outranks them. If such a crate initialises *after* the keymap, call [`rebind`] once
//! more at the end of start-up so the order is the same as on a reload.

use std::path::{Path, PathBuf};

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, Global, KeyBinding, Task, UpdateGlobal as _};
use oxikube_settings::paths::config_dir;
use oxikube_settings::watcher::{DEFAULT_DEBOUNCE, SettingsFileWatcher};

use crate::diagnostics::KeymapDiagnostic;
use crate::layer::KeymapLayer;
use crate::paths::{read_or_empty, user_keymap_path};
use crate::store::{KeymapOptions, KeymapStore};

impl Global for KeymapStore {}

/// Keeps the hot-reload watcher and its apply task alive for the life of the app.
struct KeymapWatch {
    _watcher: SettingsFileWatcher,
    _apply: Task<()>,
}

impl Global for KeymapWatch {}

/// Install the keymap from the default config dir, bind it, and start hot reload of
/// `keymap.json`. Problems are logged and leave the defaults or the last good keymap in effect;
/// they never stop start-up.
pub fn init(cx: &mut App) {
    init_with_options(KeymapOptions::default(), cx);
}

/// [`init`] with explicit options (the vim flag, an OS override).
pub fn init_with_options(options: KeymapOptions, cx: &mut App) {
    match config_dir() {
        Some(dir) => install(cx, options, Some(&dir), true),
        None => {
            tracing::warn!("no config directory; using the default keymap only");
            install(cx, options, None, false);
        }
    }
}

/// Install the keymap reading `<config_dir>/keymap.json`, without a file watcher (tests and
/// tools: GPUI's deterministic scheduler forbids the watcher's OS thread).
pub fn init_with_dir(config_dir: &Path, options: KeymapOptions, cx: &mut App) {
    install(cx, options, Some(config_dir), false);
}

/// Install the keymap with `user_text` as the contents of `keymap.json` (no file involved).
pub fn init_with_text(user_text: &str, options: KeymapOptions, cx: &mut App) {
    let mut store = KeymapStore::new(options);
    store.set_user_text(user_text);
    crate::base_keymap::apply_to_new_store(cx, &mut store);
    cx.set_global(store);
    rebind(cx);
    crate::base_keymap::follow_base_keymap(cx);
}

fn install(cx: &mut App, options: KeymapOptions, dir: Option<&Path>, watch: bool) {
    let mut store = KeymapStore::new(options);
    // The file to watch and the text it had at start-up.
    let mut watched = None;
    if let Some(dir) = dir {
        let path = user_keymap_path(dir);
        let text = load_initial(&mut store, &path);
        watched = Some((path, text));
    }
    crate::base_keymap::apply_to_new_store(cx, &mut store);
    cx.set_global(store);
    rebind(cx);
    crate::base_keymap::follow_base_keymap(cx);

    if watch && let Some((path, text)) = watched {
        start_watch(cx, path, text);
    }
}

/// Read `path` into the store's user layer and return the text the watcher should treat as
/// current. An unreadable file (UTF-16, permission denied, a directory) is recorded as a
/// diagnostic and yields empty text, so the watcher still starts and the fixed file is picked up.
fn load_initial(store: &mut KeymapStore, path: &Path) -> String {
    match read_or_empty(path) {
        Ok(text) => {
            store.set_user_text(&text);
            text
        }
        Err(err) => {
            tracing::warn!(%err, "could not read keymap.json; using the defaults");
            let reason = std::error::Error::source(&err)
                .map_or_else(|| err.to_string(), |source| format!("{err}: {source}"));
            store.set_user_unreadable(&reason);
            String::new()
        }
    }
}

fn start_watch(cx: &mut App, path: PathBuf, initial_text: String) {
    if let Some(dir) = path.parent()
        && let Err(err) = std::fs::create_dir_all(dir)
    {
        tracing::warn!(%err, "keymap hot reload is off");
        return;
    }
    let (tx, mut rx) = mpsc::unbounded::<String>();
    let watcher =
        match SettingsFileWatcher::spawn(path, initial_text, DEFAULT_DEBOUNCE, move |text| {
            tx.unbounded_send(text).is_ok()
        }) {
            Ok(watcher) => watcher,
            Err(err) => {
                tracing::warn!(%err, "keymap hot reload is off");
                return;
            }
        };
    let apply = cx.spawn(async move |cx| {
        while let Some(text) = rx.next().await {
            cx.update(|cx| reload_user_keymap(cx, &text));
        }
    });
    cx.set_global(KeymapWatch {
        _watcher: watcher,
        _apply: apply,
    });
}

/// Apply new `keymap.json` text, as the watcher does: parse, keep the previous keymap when the
/// text is not valid, otherwise rebind. Reloads that leave the sections equal rebind nothing.
pub fn reload_user_keymap(cx: &mut App, text: &str) {
    let changed = KeymapStore::update_global(cx, |store, _| store.set_user_text(text));
    apply(cx, changed);
}

/// Turn the vim layer on or off and rebind. The user-facing flag (a setting) calls this when it
/// changes.
pub fn set_vim_layer(cx: &mut App, enabled: bool) {
    let changed = KeymapStore::update_global(cx, |store, _| store.set_vim(enabled));
    apply(cx, changed);
}

/// Merge the layers again and replace the keymap's bindings. See the module docs for when to
/// call it directly.
pub fn rebind(cx: &mut App) {
    apply(cx, true);
}

fn apply(cx: &mut App, rebind: bool) {
    let merged = KeymapStore::update_global(cx, |store, cx| store.merge(cx));
    for diagnostic in cx.global::<KeymapStore>().diagnostics() {
        tracing::warn!(%diagnostic, "keymap");
    }
    if merged.skipped_embedded > 0 {
        tracing::debug!(
            skipped = merged.skipped_embedded,
            "embedded key bindings skipped: their actions are not registered in this build"
        );
    }
    if rebind {
        replace_layers(cx, merged.bindings);
    }
}

/// Replace the keymap layers in GPUI's keymap, keeping bindings other crates added.
///
/// GPUI can only clear the whole keymap, so the bindings that do not carry a layer's metadata
/// are read back first and re-added ahead of the new ones.
fn replace_layers(cx: &mut App, bindings: Vec<KeyBinding>) {
    let foreign: Vec<KeyBinding> = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|binding| KeymapLayer::from_meta(binding.meta()).is_none())
        .cloned()
        .collect();
    cx.clear_key_bindings();
    cx.bind_keys(foreign);
    cx.bind_keys(bindings);
}

/// The problems found by the last load, for a toast or the keymap editor.
pub fn diagnostics(cx: &App) -> Vec<KeymapDiagnostic> {
    cx.try_global::<KeymapStore>()
        .map(|store| store.diagnostics().to_vec())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_file_is_recorded_and_hands_the_watcher_empty_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = user_keymap_path(dir.path());
        // Invalid UTF-8, as a UTF-16 save is.
        std::fs::write(&path, [0xFF, 0xFE, b'[', 0, b']', 0]).unwrap();
        let mut store = KeymapStore::new(KeymapOptions::default());
        // `install` starts the watcher with whatever this returns, so it must not be skipped.
        assert_eq!(load_initial(&mut store, &path), "");
    }
}
