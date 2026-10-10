//! `oxikube_keymap` — layer: `platform`.
//!
//! Layered, hot-reloading key bindings in Zed's `keymap.json` format (E05-S07).
//!
//! Layers, lowest first: the embedded per-OS `default-*.json` ([`oxikube_assets::default_keymap`]),
//! the optional embedded `vim.json` (the `base_keymap: "vim"` setting, [`KeymapOptions::vim`] / [`set_vim_layer`]), then
//! the user's `keymap.json` next to `settings.json`. The layers are merged into one flat list of
//! GPUI `KeyBinding`s and bound with `cx.bind_keys`; later layers win, and `null` unbinds.
//!
//! A feature crate takes part by:
//! 1. declaring its actions (`actions!(table, [SelectNext])` or `#[derive(Action)]
//!    #[action(namespace = table)]` for actions with data), which registers them by name,
//! 2. giving its views a key context ([`KeyContextual`], names in [`contexts`]),
//! 3. adding default bindings to the per-OS files in `oxikube_assets/assets/keymaps/`. An entry
//!    whose action is not registered in a build is skipped silently in the embedded layers and
//!    reported in the user's file.
//!
//! An action whose name is a declared `CommandId` *is* that command: the palette, the key and
//! the MCP tool run one behaviour ([`ActionRegistry::command`]).
//!
//! Module map:
//! - [`base_keymap`]: the `base_keymap` setting (`default` | `vim`) that drives the vim layer.
//! - [`mod@file`]: the `keymap.json` format and its lenient parser.
//! - [`registry`]: [`ActionRegistry`], names by namespace and the action-to-`Command` mapping.
//! - [`build`]: sections to `KeyBinding`s, with validation.
//! - [`store`]: [`KeymapStore`], the layers and their merge; [`layer`], [`mod@diagnostics`].
//! - [`global`]: [`init`], hot reload, [`rebind`]; [`paths`]: where `keymap.json` lives.
//! - [`context`]: key-context helpers and the standard context names.
//! - [`query`]: the bindings of an action or a command, for the palette.
//! - [`mod@dispatch`]: what a key does in a context stack written as data, and the bindings in
//!   force there (tests, the help overlay).
//! - [`stands_for`]: which commands the view actions (`resource_table::ViewYaml`) stand for.
//!
//! The design (sections, `null`, context predicates, layering) follows Zed's `keymap_file.rs`;
//! the code is written from scratch, so no Zed licence header applies.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod base_keymap;
pub mod build;
pub mod context;
pub mod diagnostics;
pub mod dispatch;
pub mod file;
pub mod global;
pub mod layer;
pub mod paths;
pub mod query;
pub mod registry;
pub mod stands_for;
pub mod store;

pub use base_keymap::{BaseKeymap, KeymapSettings, KeymapSettingsContent};
pub use context::{KeyContextBuilder, KeyContextual, contexts};
pub use diagnostics::{KeymapDiagnostic, KeymapProblem};
pub use dispatch::{ActiveBinding, Resolution, active_bindings, parse_stack, resolve};
pub use file::{KeymapAction, KeymapSection};
pub use global::{
    diagnostics, init, init_with_dir, init_with_options, init_with_text, rebind,
    reload_user_keymap, set_vim_layer,
};
pub use layer::KeymapLayer;
pub use oxikube_assets::KeymapPlatform;
pub use query::{BindingInfo, bindings_for_action, bindings_for_action_name, bindings_for_command};
pub use registry::ActionRegistry;
pub use store::{KeymapOptions, KeymapStore};
