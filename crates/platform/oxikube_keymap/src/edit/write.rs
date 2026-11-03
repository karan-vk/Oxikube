//! Writing an edit to the user's `keymap.json` and putting it in force at once.
//!
//! The file is read, edited, validated and written on the background executor, under a lock that
//! serialises the app's own edits; the UI thread then applies the written text as the new user
//! layer ([`crate::reload_user_keymap`]), so the new binding works before the file watcher has
//! seen anything. When the watcher does report the write, the sections are the ones already in
//! force: the reload changes nothing and rebinds nothing, so the app's own write never loops.

use std::path::Path;
use std::sync::Mutex;

use gpui::{App, AppContext as _, Task};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_settings::paths::write_atomically;

use super::gate::preview_edit;
use super::ops::KeybindUpdateOperation;
use crate::known::KnownNames;
use crate::paths::read_or_empty;
use crate::registry::ActionRegistry;
use crate::store::KeymapStore;

/// Serialises the app's edits of `keymap.json` (two saves in a row must not both start from the
/// text before either).
static EDIT_LOCK: Mutex<()> = Mutex::new(());

/// The text the user layer was last loaded from (empty without a user file), for a preview.
pub fn user_text(cx: &App) -> String {
    cx.try_global::<KeymapStore>()
        .and_then(KeymapStore::user_text)
        .unwrap_or_default()
        .to_owned()
}

/// Apply `operations` to the user's `keymap.json`, keeping its comments, and put the result in
/// force. Without a user file (the keymap was installed from text) the in-memory user layer is
/// edited instead.
///
/// The returned task fails, and nothing is written, when the file cannot be edited (it is not a
/// list of sections, the binding to change is gone) or when the edited text has an error the file
/// did not have.
pub fn write_user_keymap(
    cx: &mut App,
    operations: Vec<KeybindUpdateOperation>,
) -> Task<OxiResult<()>> {
    let Some(store) = cx.try_global::<KeymapStore>() else {
        return Task::ready(Err(OxiError::internal("the keymap is not installed")));
    };
    let actions = ActionRegistry::from_app(cx);
    let known = KnownNames::current();
    let Some(path) = store.user_path().map(Path::to_path_buf) else {
        let old = store.user_text().unwrap_or_default().to_owned();
        let result = edited_text(&old, &operations, &actions, &known).map(|text| {
            crate::global::reload_user_keymap(cx, &text);
        });
        return Task::ready(result);
    };
    let write = cx.background_spawn(async move {
        // A poisoned lock only means another edit panicked; the file is still whole (atomic).
        let _guard = EDIT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let old = read_or_empty(&path)?;
        let new = edited_text(&old, &operations, &actions, &known)?;
        if new != old {
            write_atomically(&path, &new)?;
        }
        Ok::<_, OxiError>(new)
    });
    cx.spawn(async move |cx| {
        let text = write.await?;
        cx.update(|cx| crate::global::reload_user_keymap(cx, &text));
        Ok(())
    })
}

/// The edited text, or why it must not be written.
fn edited_text(
    old: &str,
    operations: &[KeybindUpdateOperation],
    actions: &ActionRegistry,
    known: &KnownNames,
) -> OxiResult<String> {
    let preview = preview_edit(old, operations, actions, known)
        .map_err(|err| OxiError::validation(err.to_string()))?;
    if let Some(first) = preview.introduced.first() {
        return Err(OxiError::validation(format!(
            "the edit would make keymap.json invalid, so it was not written: {}",
            first.message()
        )));
    }
    Ok(preview.text)
}
