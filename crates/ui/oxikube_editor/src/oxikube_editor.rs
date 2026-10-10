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

pub mod validate;
pub mod view;
pub mod yaml;

/// Registers the editor's actions (`editor::NewManifest`). Call once at start-up, after
/// `oxikube_ui::init`.
pub fn init(cx: &mut gpui::App) {
    view::init(cx);
}
