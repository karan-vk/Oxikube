//! GPUI wiring: install the [`KeymapStore`] global, bind the merged layers, and hot reload
//! `keymap.json`.
//!
//! Start-up reads the (small, local) user file synchronously so the first frame already has the
//! user's bindings. After that the watcher thread reads the file *and parses it* (JSON with
//! comments to sections, with their lines, [`ParsedUserKeymap`]); the UI thread only checks each
//! binding against the action registry (which needs the app) and swaps the keymap in one call, a
//! fraction of a millisecond per ten bindings (see the `reload_cost` test). Keystroke dispatch
//! itself is GPUI's and allocates nothing here.
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
use oxikube_domain::{OxiError, OxiResult};
use oxikube_settings::paths::config_dir;
use oxikube_settings::watcher::{DEFAULT_DEBOUNCE, SettingsFileWatcher};

use crate::conflicts::KeymapConflict;
use crate::diagnostics::KeymapDiagnostic;
use crate::layer::KeymapLayer;
use crate::paths::{read_or_empty, user_keymap_path};
use crate::store::{KeymapOptions, KeymapStore, ParsedUserKeymap};

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
    install_store(cx, store);
}

/// Make `store` the keymap global, bind it and follow the `base_keymap` setting.
fn install_store(cx: &mut App, mut store: KeymapStore) {
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
        store.set_user_path(Some(path.clone()));
        watched = Some((path, text));
    }
    install_store(cx, store);

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
            store.set_user_unreadable(&unreadable_reason(&err));
            String::new()
        }
    }
}

/// The error and its cause on one line, for the diagnostic of an unreadable file.
fn unreadable_reason(err: &OxiError) -> String {
    std::error::Error::source(err)
        .map_or_else(|| err.to_string(), |source| format!("{err}: {source}"))
}

/// What the watcher's thread hands to the UI thread after a settled change of the file.
enum Update {
    /// The new text, parsed on the watcher's thread.
    Parsed(ParsedUserKeymap),
    /// The file could not be read (saved as UTF-16, permissions changed, it became a directory).
    Unreadable(String),
}

/// The watcher's callback: parse each new text off the UI thread and forward it, or forward the
/// reason a read failed. Returns `false` once the receiver is gone, which stops the watcher.
fn forward(
    tx: mpsc::UnboundedSender<Update>,
) -> impl FnMut(OxiResult<String>) -> bool + Send + 'static {
    move |read| {
        let update = match read {
            Ok(text) => Update::Parsed(ParsedUserKeymap::parse(&text)),
            Err(err) => {
                tracing::warn!(%err, "could not read keymap.json; the previous keymap stays");
                Update::Unreadable(unreadable_reason(&err))
            }
        };
        tx.unbounded_send(update).is_ok()
    }
}

fn start_watch(cx: &mut App, path: PathBuf, initial_text: String) {
    if let Some(dir) = path.parent()
        && let Err(err) = std::fs::create_dir_all(dir)
    {
        tracing::warn!(%err, "keymap hot reload is off");
        return;
    }
    let (tx, mut rx) = mpsc::unbounded::<Update>();
    // The callback runs on the watcher's thread: the parse costs nothing the UI thread sees.
    let watcher = match SettingsFileWatcher::spawn_reporting(
        path,
        initial_text,
        DEFAULT_DEBOUNCE,
        forward(tx),
    ) {
        Ok(watcher) => watcher,
        Err(err) => {
            tracing::warn!(%err, "keymap hot reload is off");
            return;
        }
    };
    let apply = cx.spawn(async move |cx| {
        while let Some(update) = rx.next().await {
            cx.update(|cx| apply_update(cx, update));
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
    apply_update(cx, Update::Parsed(ParsedUserKeymap::parse(text)));
}

fn apply_update(cx: &mut App, update: Update) {
    let changed = KeymapStore::update_global(cx, |store, _| match update {
        Update::Parsed(parsed) => store.set_user_parsed(parsed),
        Update::Unreadable(message) => {
            store.set_user_unreadable(&message);
            false
        }
    });
    apply(cx, changed);
}

/// Read `keymap.json` again and apply it, as the watcher does after a save: the explicit reload
/// for tests and tools, which run without a watcher. It reads the file on the calling thread, so
/// the UI never calls it. Does nothing when the keymap has no file (installed from text).
pub fn reload(cx: &mut App) {
    let Some(path) = cx
        .try_global::<KeymapStore>()
        .and_then(|store| store.user_path().map(Path::to_path_buf))
    else {
        return;
    };
    match read_or_empty(&path) {
        Ok(text) => reload_user_keymap(cx, &text),
        Err(err) => apply_update(cx, Update::Unreadable(unreadable_reason(&err))),
    }
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
    for conflict in cx.global::<KeymapStore>().conflicts() {
        tracing::info!(
            layer = %conflict.layer,
            context = conflict.context.as_deref().unwrap_or("(everywhere)"),
            keystrokes = %conflict.keystrokes,
            winner = %conflict.winner().action,
            "the same key is bound more than once; the last binding wins"
        );
    }
    let user = cx.global::<KeymapStore>().user_diagnostics();
    crate::events::publish(cx, user);
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

/// The problems found by the last load, in every layer, for the keymap editor.
pub fn diagnostics(cx: &App) -> Vec<KeymapDiagnostic> {
    cx.try_global::<KeymapStore>()
        .map(|store| store.diagnostics().to_vec())
        .unwrap_or_default()
}

/// The problems with the user's `keymap.json` found by the last load: what the notification
/// lists. Read it after subscribing ([`crate::subscribe_diagnostics`]) to catch the start-up load.
pub fn user_diagnostics(cx: &App) -> Vec<KeymapDiagnostic> {
    cx.try_global::<KeymapStore>()
        .map(KeymapStore::user_diagnostics)
        .unwrap_or_default()
}

/// Keys bound more than once, to different things, in one context of one layer (the later
/// binding wins): what the help overlay lists as conflicts.
pub fn conflicts(cx: &App) -> Vec<KeymapConflict> {
    cx.try_global::<KeymapStore>()
        .map(|store| store.conflicts().to_vec())
        .unwrap_or_default()
}

/// The user's `keymap.json`, when the keymap was installed from a config directory (the file may
/// not exist yet; [`crate::ensure_user_keymap`] creates it).
pub fn user_keymap_file(cx: &App) -> Option<PathBuf> {
    cx.try_global::<KeymapStore>()?
        .user_path()
        .map(Path::to_path_buf)
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

    #[test]
    fn the_watchers_callback_forwards_parsed_text_and_read_failures() {
        let (tx, mut rx) = mpsc::unbounded();
        let mut callback = forward(tx);
        assert!(callback(Ok("[]".into())));
        let err = OxiError::internal("could not read keymap.json")
            .with_source(std::io::Error::other("stream did not contain valid UTF-8"));
        assert!(callback(Err(err)));
        assert!(matches!(rx.try_recv(), Ok(Update::Parsed(_))));
        let Ok(Update::Unreadable(message)) = rx.try_recv() else {
            panic!("expected the failure to be forwarded");
        };
        assert!(message.contains("valid UTF-8"), "{message}");
        drop(rx);
        assert!(
            !callback(Ok(String::new())),
            "a gone receiver stops the watcher"
        );
    }
}
