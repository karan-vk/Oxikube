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
//! - [`aliases`]: the user's `aliases.json` (E11-S04): parse with line-numbered diagnostics, hot reload.
//! - [`audit`]: the `audit` setting, how many days the audit log keeps its records (E19-S08).
//! - [`cluster`]: the per-cluster settings (`clusters.<id>`: name, colour, read-only, ...), E06-S08.
//! - [`cloud`]: the `cloud` setting, providers, CLI paths, scan lists, cache TTL, parallelism (E18-S07).
//! - [`kustomize`]: the `kustomize` setting, the binary that builds overlays (E20-S04).
//! - [`diff`]: the `diff` setting, the paths the cross-cluster resource diff ignores (E20-S09).
//! - [`helm`]: the `helm` setting, binary path, server-side apply, default repositories (E16-S11).
//! - [`kubeconfig`]: the `kubeconfig.sources` setting, the kubeconfig files and folders to read (E06-S05).
//! - [`metrics`]: the `metrics` setting, which extended resources count as GPUs (E13-S02).
//! - [`portforward`]: the `portforward` setting, default bind, read-only policy, port range (E15-S08).
//! - [`settings`]: the [`Settings`] trait, [`SettingsLocation`], registration.
//! - [`store`]: [`SettingsStore`], layer merge, per-setting values and change tracking.
//! - [`global`]: GPUI global, [`init`], startup load, hot reload task, file edits.
//! - [`paths`]: config dir (`$OXIKUBE_CONFIG_DIR`), first-run `settings.json`, atomic writes.
//! - [`watcher`]: `notify` watcher with debounce (no GPUI).
//! - [`update`]: typed edit → comment-preserving text edit.
//! - [`json_edit`]: vendored Zed `settings_json` text edits (GPL header).
//! - [`jsonc`], [`diagnostics`], [`schema`]: parsing, problems found on load, JSON schema.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod aliases;
pub mod audit;
pub mod cloud;
pub mod cluster;
pub mod diagnostics;
pub mod diff;
pub mod global;
pub mod helm;
pub mod json_edit;
pub mod jsonc;
pub mod kubeconfig;
pub mod kustomize;
pub mod metrics;
pub mod paths;
pub mod portforward;
pub mod schema;
pub mod schema_dynamic;
pub mod settings;
pub mod store;
#[cfg(test)]
mod test_support;
pub mod update;
pub mod watcher;

pub use audit::{AuditSettings, AuditSettingsContent};
pub use cloud::{CloudSettings, CloudSettingsContent};
pub use cluster::{ClusterSettings, ClusterSettingsContent};
pub use diagnostics::SettingsDiagnostic;
pub use diff::{DiffSettings, DiffSettingsContent};
pub use global::{init, init_with_dir, update_user_settings};
pub use helm::{HelmRepoContent, HelmSettings, HelmSettingsContent};
pub use kubeconfig::{KubeconfigSettings, KubeconfigSettingsContent};
pub use kustomize::{KustomizeSettings, KustomizeSettingsContent};
pub use metrics::{
    DEFAULT_HISTORY_REFRESH_SECS, MAX_HISTORY_REFRESH_SECS, MIN_HISTORY_REFRESH_SECS,
    MetricsSettings, MetricsSettingsContent,
};
pub use portforward::{PortForwardSettings, PortForwardSettingsContent};
pub use schema_dynamic::DynamicEnums;
pub use settings::{RegisteredSetting, Settings, SettingsContent, SettingsLocation};
pub use store::SettingsStore;
pub use update::new_text_for_update;

/// Re-exports used by [`register_settings!`]; not part of the API.
#[doc(hidden)]
pub mod private {
    pub use inventory;
}
