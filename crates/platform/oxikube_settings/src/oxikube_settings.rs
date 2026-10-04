//! `oxikube_settings` — layer: `platform`.
//!
//! Typed, layered, hot-reloading settings (Zed's settings design, E05-S06).
//!
//! Layers, lowest first: the embedded `default.json` ([`oxikube_assets::default_settings`]),
//! the user's `settings.json` (JSON with comments), then `clusters.<id>` overrides inside the
//! user file. A feature crate owns its settings:
//!
//! 1. a serde content struct (`Option` fields, `JsonSchema`) for one key of the file,
//! 2. a runtime struct implementing [`Settings`] (`KEY`, `Content`, `from_content`),
//! 3. [`register_settings!`] so [`SettingsStore::new`] picks it up through `inventory`,
//! 4. defaults in `default.json` and a regenerated `settings.schema.json`
//!    (`cargo xtask gen-settings-schema`).
//!
//! Reads are `T::get_global(cx)` / `T::get(Some(location), cx)` (references, no parsing);
//! [`Settings::observe`] fires only when `T`'s resolved value changes; edits go through
//! [`update_user_settings`], which rewrites only the changed values of the file.
//!
//! Module map:
//! - [`settings`]: the [`Settings`] trait, [`SettingsLocation`], registration.
//! - [`store`]: [`SettingsStore`], layer merge, per-setting values and change tracking.
//! - [`global`]: GPUI global, [`init`], startup load, hot reload task, file edits.
//! - [`paths`]: config dir (`$OXIKUBE_CONFIG_DIR`), first-run `settings.json`.
//! - [`watcher`]: `notify` watcher with debounce (no GPUI).
//! - [`update`]: typed edit → comment-preserving text edit.
//! - [`json_edit`]: vendored Zed `settings_json` text edits (GPL header).
//! - [`jsonc`], [`diagnostics`], [`schema`]: parsing, problems found on load, JSON schema.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod diagnostics;
pub mod global;
pub mod json_edit;
pub mod jsonc;
pub mod paths;
pub mod schema;
pub mod settings;
pub mod store;
#[cfg(test)]
mod test_support;
pub mod update;
pub mod watcher;

pub use diagnostics::SettingsDiagnostic;
pub use global::{init, init_with_dir, update_user_settings};
pub use settings::{RegisteredSetting, Settings, SettingsContent, SettingsLocation};
pub use store::SettingsStore;
pub use update::new_text_for_update;

/// Re-exports used by [`register_settings!`]; not part of the API.
#[doc(hidden)]
pub mod private {
    pub use inventory;
}
