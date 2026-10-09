//! `oxikube_theme` — layer: `platform`.
//!
//! Themes (E05-S08): a Zed theme-family JSON (schema v0.2.0) importer into [`ThemeTokens`], the
//! `oxikube` block of Kubernetes status colours, a [`ThemeRegistry`] (bundled One Dark / One
//! Light plus the user's hot-reloaded `themes/` directory), system-appearance following and the
//! `theme` setting. Only Zed's file *format* is shared; its GPL `theme` crate is not copied.
//!
//! Module map:
//! - [`tokens`]: [`ThemeTokens`], the neutral colour description of one theme (only
//!   `gpui::Hsla` from GPUI, so it is `Send + Sync`).
//! - [`import`]: Zed theme-family JSON -> tokens through a table-driven mapper; lenient JSON,
//!   defaults for unset keys, an [`ImportReport`] for what was wrong.
//! - [`registry`]: [`ThemeRegistry`], `get` / `list` / `resolve(selection, system)`; pure.
//! - [`settings`]: the `theme` setting (`"Ayu Dark"` or `{ mode, light, dark }`).
//! - [`appearance`]: [`Appearance`], [`ThemeMode`] and the [`SystemAppearance`] global.
//! - [`user_dir`], [`watcher`]: scanning `<config>/themes/` and `notify` hot reload (the thread
//!   does the file reads and parsing, never the UI thread).
//! - [`global`]: [`init`], [`ActiveTheme`] and the observers that keep it current.
//! - [`color`]: `#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa` <-> `Hsla`.
//! - [`glyph_warm`]: the glyph atlas warmed for every installed theme's text colours, so a theme
//!   switch draws without rasterising text in the frame (E05-P602).
//!
//! `oxikube_ui` turns the active [`ThemeTokens`] into gpui-component's theme (`ThemeConfig`)
//! in its `theme_bridge`, since only that crate may import gpui-component.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod appearance;
pub mod color;
pub mod global;
pub mod glyph_warm;
pub mod import;
pub mod registry;
pub mod settings;
pub mod tokens;
pub mod user_dir;
pub mod watcher;

#[cfg(test)]
mod test_fixtures;

pub use appearance::{Appearance, SystemAppearance, ThemeMode};
pub use global::{
    ActiveTheme, apply_user_scan, init, init_watching_dir, init_with_dir, refresh_active,
};
pub use import::{
    ImportDiagnostic, ImportError, ImportReport, ImportedFamily, ThemeFamily, import_family,
};
pub use registry::{ThemeFileProblem, ThemeMeta, ThemeRegistry, ThemeSource};
pub use settings::{ThemeSelection, ThemeSelectionContent, ThemeSettings};
pub use tokens::ThemeTokens;
