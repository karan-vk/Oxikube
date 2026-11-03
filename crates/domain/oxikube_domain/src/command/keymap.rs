//! The payloads of the keymap editing commands (E21-S08): `keymap::SetBinding` and
//! `keymap::RemoveBinding`.
//!
//! Both write the user's `keymap.json` only (never a default keymap, never a cluster), so they
//! are not mutations and carry no guard tier. The keymap crate turns them into comment-preserving
//! edits of the file and refuses one whose result does not validate.

use serde::{Deserialize, Serialize};

/// Which keymap a binding is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeymapSource {
    /// The embedded per-OS default keymap. It is never edited: changing one of its bindings
    /// writes an override, or a `null` that unbinds it, to the user's file.
    Default,
    /// The base keymap layered over the defaults (vim). Never edited either.
    Base,
    /// The user's `keymap.json`.
    User,
}

/// A binding as the keymap editor lists it: where it is, not what it does.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeymapBindingRef {
    /// The context expression of its section, as the editor shows it; `None` for everywhere.
    #[serde(default)]
    pub context: Option<String>,
    /// Its keystrokes, space separated (`g g`, `cmd-k cmd-s`).
    pub keystrokes: String,
    /// The keymap it is in.
    pub source: KeymapSource,
}

/// What `keymap::SetBinding` writes: `keystrokes` in `context` run `action`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeymapBindingEdit {
    /// The action's registered name (`resource_table::ViewYaml`, or a command id).
    pub action: String,
    /// The action's JSON data as text, for an action that takes data.
    #[serde(default)]
    pub data: Option<String>,
    /// The context expression the binding is limited to; `None` for everywhere.
    #[serde(default)]
    pub context: Option<String>,
    /// The keystrokes, space separated, with the platform's own modifier names (`cmd-k` on
    /// macOS, `ctrl-k` elsewhere).
    pub keystrokes: String,
    /// The binding this one replaces: edited in place when it is the user's, otherwise
    /// overridden (and unbound with `null` when its keys or context differ).
    #[serde(default)]
    pub replaces: Option<KeymapBindingRef>,
    /// Bindings to take out of force too, the conflicts the user chose to replace: a user binding
    /// is deleted, a default one gets a `null` in the user's file.
    #[serde(default)]
    pub unbind: Vec<KeymapBindingRef>,
}
