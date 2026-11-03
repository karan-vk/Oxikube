//! Editing the user's `keymap.json` from the keymap editor (E21-S08): the edit operations, the
//! conflict check across the context chain, and the validated, comment-preserving write.
//!
//! | Module | Holds |
//! |---|---|
//! | `ops` | [`KeybindUpdateOperation`] (`Add`, `Replace`, `Remove`) and [`apply_operations`]: text in, text out, on the vendored array helpers of `oxikube_settings::json_edit` |
//! | `plan` | the `keymap::SetBinding` / `keymap::RemoveBinding` payloads to operations, and rows to payloads |
//! | `chain` | [`ContextModel`]: which bindings in force a new one clashes with (same keys or a prefix, contexts that can be active together) |
//! | `gate` | [`preview_edit`]: the edited text through the one validator; an edit that adds an error is never written |
//! | `write` | [`write_user_keymap`]: read, edit, validate and write off the UI thread, then put the result in force |
//! | `keystrokes` | [`keystroke_text`]: a recorded keystroke with the platform's own modifier names |
//!
//! Default and base keymaps are never edited: an override or a `null` goes to the user's file.
//! The design follows Zed's keymap editor (`KeybindUpdateOperation`); the code is written from
//! scratch, so no Zed licence header applies. The JSON text edits it builds on are the vendored
//! Zed functions in `oxikube_settings::json_edit`, which carry theirs.

mod chain;
mod gate;
mod keystrokes;
mod ops;
mod plan;
mod text;
mod write;

#[cfg(test)]
mod tests;

pub use chain::{ChainConflict, ConflictKind, ContextModel};
pub use gate::{EditPreview, introduced_errors, preview_edit};
pub use keystrokes::{is_modifier_only, keystroke_text, sequence_text};
pub use ops::{
    BindingLocation, EditError, KeybindTarget, KeybindUpdateOperation, action_value,
    apply_operations, normalise_context,
};
pub use plan::{
    PlanError, layer_of, location_of, ref_of_row, remove_binding_operations,
    set_binding_operations, source_of,
};
pub use write::{user_text, write_user_keymap};
