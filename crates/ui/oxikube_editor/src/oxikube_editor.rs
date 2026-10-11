//! `oxikube_editor` — layer: `ui`.
//!
//! Manifest editor: YAML/JSON on gpui-component editor, spanned YAML (granit-parser), OpenAPI schema validation, hover/completion, diff vs live, dry-run + SSA apply, templates.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

//! | Module | Story | What |
//! |---|---|---|
//! | [`yaml`] | E10-S02 | the spanned YAML model (granit-parser) |
//! | [`validate`] | E10-S03 | schema validation into diagnostics |
//! | [`view`] | E10-S04 | [`view::ManifestEditor`], the `editor::*` commands |
//! | [`settings`] | E10-S12 | the `editor` settings: tab size, font, wrap, validation, masking, field manager |
//! | [`json`] | E10-S10 | JSON mode: [`json::Language`], the strict JSON check, YAML to JSON conversion |
//! | [`open`] | E10-S06 | an editor opened from a live object: [`open::render_resource`], Secret masking, the `resource::Edit` row actions |
//! | [`diff`] | E10-S07 | diff vs live, the three-way merge, the changed-on-server state machine |
//! | [`cancel`] | E10-S11 | [`cancel::CancelToken`]: how a stale parse or validation stops early |

pub mod cancel;
pub mod diff;
pub mod json;
pub mod open;
pub mod settings;
pub mod validate;
pub mod view;
pub mod yaml;

/// Registers the editor's actions (`editor::NewManifest`). Call once at start-up, after
/// `oxikube_ui::init`.
pub fn init(cx: &mut gpui::App) {
    view::init(cx);
}
