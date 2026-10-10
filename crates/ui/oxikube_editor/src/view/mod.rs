//! The manifest editor view (E10-S04): [`ManifestEditor`], a workspace item over
//! `oxikube_ui::editor::CodeEditor` that validates its YAML against the cluster's schemas, and
//! the `editor::*` commands that open and drive it.
//!
//! | Module | What |
//! |---|---|
//! | `manifest_editor` | [`ManifestEditor`]: the view, its key context (`ManifestEditor`, `mode == yaml`, `Editing`), the debounced validation off the UI thread and the schema fetches |
//! | `model` | [`ManifestModel`]: the logic over `EditorApi` only (stale results, schemas to fetch, problem counts, toggles), tested without a window |
//! | `validation` | [`validate_text`]: one pass of the YAML model and the validator, as editor diagnostics |
//! | `schemas` | [`SchemaSource`]: the cluster's `SchemaPort` ([`SessionSchemas`] in the app) |
//! | `toolbar` | what the buffer is checked against, the problem count, the Read-only and Wrap toggles |
//! | `item` | the workspace `Item`: tab title, icon, dirty dot; never saved with the layout |
//! | `commands` | the bus handlers ([`register_commands`]): queue an [`EditorRequest`] on the window's [`EditorViewSink`] |
//! | `controller` | [`EditorViews`]: applies the requests in the shown workspace through an [`EditorHost`] ([`ClusterEditorHost`] in the app) |
//!
//! The editor never writes its buffer anywhere: not to the layout, not to logs. Applying it to a
//! cluster is `resource::Apply` (E10-S08), through the `MutationGuard`.

mod commands;
mod controller;
mod item;
mod manifest_editor;
mod model;
mod schemas;
mod toolbar;
mod validation;

use std::rc::Rc;

use gpui::{App, Global, actions};
use oxikube_domain::command::Command;
use oxikube_workspace::CommandDispatcher;

pub use commands::{EDITOR_COMMANDS, EditorRequest, EditorViewSink, register_commands};
pub use controller::{ClusterEditorHost, EditorHost, EditorViews, EditorViewsDeps};
pub use manifest_editor::{ManifestEditor, ManifestEditorParts};
pub use model::{Accepted, ManifestModel, Problems};
pub use schemas::{SchemaSource, SessionSchemas};
pub use validation::{KnownSchemas, VALIDATION_DEBOUNCE, Validation, validate_text};

actions!(
    editor,
    [
        /// Open an empty manifest editor (`editor::NewManifest` on the bus).
        NewManifest,
        /// Make the focused manifest editor read-only, or editable again
        /// (`editor::ToggleReadOnly`).
        ToggleReadOnly,
        /// Wrap the focused manifest editor's long lines, or not (`editor::ToggleSoftWrap`).
        ToggleSoftWrap,
    ]
);

/// Where the global `editor::NewManifest` action sends its command.
struct EditorDispatcher(Rc<dyn CommandDispatcher>);

impl Global for EditorDispatcher {}

/// Registers the global `editor::NewManifest` action (it dispatches the bus command through the
/// dispatcher [`install`] set). Called by [`crate::init`]. The toggles are handled by the focused
/// [`ManifestEditor`] itself.
pub(crate) fn init(cx: &mut App) {
    cx.on_action(|_: &NewManifest, cx| {
        if let Some(dispatcher) = cx.try_global::<EditorDispatcher>().map(|d| d.0.clone()) {
            dispatcher.dispatch(Command::EditorNewManifest { cluster: None }, cx);
        }
    });
}

/// Makes `dispatcher` (the bus) where the keymap's `editor::NewManifest` goes. Call it when the
/// window mounts.
pub fn install(dispatcher: Rc<dyn CommandDispatcher>, cx: &mut App) {
    cx.set_global(EditorDispatcher(dispatcher));
}
