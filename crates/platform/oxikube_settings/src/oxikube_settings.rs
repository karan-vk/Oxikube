//! `oxikube_settings` — layer: `platform`.
//!
//! SettingsStore: default.json -> user settings.json (JSONC) -> per-cluster overrides, Settings trait + inventory registration, comment-preserving edits, schemars schema, notify hot reload (vendored Zed design).
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
