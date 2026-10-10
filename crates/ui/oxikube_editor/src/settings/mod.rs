//! The manifest editor's settings (E10-S12): the `editor` object of `settings.json`.
//!
//! ```json
//! "editor": {
//!   "tab_size": 2,
//!   "font_family": null,
//!   "font_size": null,
//!   "soft_wrap": false,
//!   "show_managed_fields": false,
//!   "mask_secrets": true,
//!   "validation": true,
//!   "field_manager": "oxikube"
//! }
//! ```
//!
//! The defaults live in `assets/settings/default.json` (with comments) and the schema is generated
//! from [`EditorSettingsContent`] (`settings.schema.json`, `cargo xtask gen-settings-schema`), so
//! `settings.json` gets completion and validation for every key.
//!
//! | Key | Read by | Applies |
//! |---|---|---|
//! | `tab_size`, `font_family`, `font_size`, `soft_wrap` | the editor view | at once to every open editor, without touching its text or undo history |
//! | `validation` | the editor view | at once: off clears the squiggles, on checks the buffer again |
//! | `show_managed_fields`, `mask_secrets` | opening a live object (E10-S06) | to the editors opened afterwards |
//! | `field_manager` | the apply flow (E10-S08) | on the next apply |
//!
//! **Per cluster.** Like every setting the block can be overridden under `clusters.<id>.editor`;
//! the editors of that cluster read the override ([`EditorSettings::resolve`] takes the cluster).
//! `field_manager` is the one a cluster most often wants its own (an installation that already
//! uses another manager name); the rest are habits of the person and are normally set at the top
//! level.
//!
//! **No secrets.** `mask_secrets: false` shows a Secret's values in the editor, in memory: decoded
//! Secret data is never written to disk, logs or the audit trail, whatever this says.
//!
//! **Invalid values** (a negative `tab_size`, a `field_manager` with a space or of the wrong
//! type) are replaced by the default for that field alone and named in
//! [`EditorSettings::problems`]; the window's [`EditorViews`](crate::view::EditorViews) shows them
//! as a notice.

mod apply;
mod content;
pub mod notice;
mod resolved;
#[cfg(test)]
mod tests;

pub use apply::apply_changes;
pub use content::EditorSettingsContent;
pub use resolved::{
    DEFAULT_FIELD_MANAGER, EditorSettings, MAX_FIELD_MANAGER_LEN, MAX_FONT_SIZE, MAX_TAB_SIZE,
    MIN_FONT_SIZE, MIN_TAB_SIZE, valid_field_manager,
};
