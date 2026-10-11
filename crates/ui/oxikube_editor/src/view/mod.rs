//! The manifest editor view (E10-S04): [`ManifestEditor`], a workspace item over
//! `oxikube_ui::editor::CodeEditor` that validates its YAML against the cluster's schemas, and
//! the `editor::*` commands that open and drive it.
//!
//! | Module | What |
//! |---|---|
//! | `manifest_editor` | [`ManifestEditor`]: the view, its key context (`ManifestEditor`, `mode == yaml`, `Editing`), and the hover docs and completion ([`crate::assist::ManifestAssist`], fed with each schema as it arrives and each validation's parse) |
//! | `pipeline` | the debounced, cancellable validation off the UI thread (a large buffer in chunks, visible objects first) and the schema fetches (E10-S11) |
//! | `model` | [`ManifestModel`]: the logic over `EditorApi` only (stale results, schemas to fetch, problem counts, coverage, the parse cache, [`PassStats`]), tested without a window |
//! | `progressive` | `plan`, `check_chunk`, `Progress`: a pass split into chunks of objects, pure (E10-S11) |
//! | `validation` | [`validate_text`] / [`validate_text_as`]: one pass of the YAML model and the validator (and, for JSON, the strict JSON check), as editor diagnostics |
//! | `schemas` | [`SchemaSource`]: the cluster's `SchemaPort` ([`SessionSchemas`] in the app) |
//! | `toolbar` | what the buffer is checked against, the problem count, the Read-only and Wrap toggles |
//! | `item` | the workspace `Item`: tab title, icon, dirty dot; never saved with the layout |
//! | `commands` | the bus handlers ([`register_commands`]): queue an [`EditorRequest`] on the window's [`EditorViewSink`] (including `editor::TriggerCompletion` and `ShowHover`, E10-S05) |
//! | `convert` | the YAML to JSON conversion (one undo step, off the UI thread) and the language switch (E10-S10) |
//! | `notice` | the one-line bar that says what a conversion did and could not carry over, with Undo |
//! | `live` | diff vs live (E10-S07): the watch on the edited object, the changed-on-server banner (Merge / Reload / Keep mine), the diff side by side or inline in the bottom panel ([`LiveSource`], [`SessionLive`]) |
//! | `controller` | [`EditorViews`]: applies the requests in the shown workspace through an [`EditorHost`] ([`ClusterEditorHost`] in the app) |
//!
//! The editor never writes its buffer anywhere: not to the layout, not to logs. Applying it to a
//! cluster is `resource::Apply` (E10-S08), through the `MutationGuard`.

mod commands;
mod controller;
mod convert;
mod item;
mod live;
mod manifest_editor;
mod model;
mod notice;
mod pipeline;
mod progressive;
mod schemas;
mod toolbar;
mod validation;

use std::rc::Rc;

use gpui::{App, Global, actions};
use oxikube_domain::command::Command;
use oxikube_workspace::CommandDispatcher;

pub use commands::{EDITOR_COMMANDS, EditorRequest, EditorViewSink, register_commands};
pub use controller::{ClusterEditorHost, EditorHost, EditorViews, EditorViewsDeps};
pub use live::{DIFF_DEBOUNCE, DiffLayout, LiveSource, SessionLive};
pub use manifest_editor::{ManifestEditor, ManifestEditorParts};
pub use model::{Accepted, ManifestModel, PassStats, Problems};
pub use notice::{Notice, NoticeLevel};
pub use progressive::{Coverage, LARGE_BUFFER_BYTES};
pub use schemas::{SchemaSource, SessionSchemas};
pub use validation::{
    KnownSchemas, VALIDATION_DEBOUNCE, Validation, validate_text, validate_text_as,
    validate_text_in, validate_text_with,
};

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
        /// Show or hide `metadata.managedFields` in the focused editor of a resource
        /// (`editor::ToggleManagedFields`).
        ToggleManagedFields,
        /// Show the values of the Secret in the focused editor, or mask them again
        /// (`editor::ToggleSecretMask`).
        ToggleSecretMask,
        /// Replace the focused manifest editor's YAML with the same data as JSON
        /// (`editor::ConvertToJson`).
        ConvertToJson,
        /// Replace the focused manifest editor's JSON with the same data as YAML
        /// (`editor::ConvertToYaml`).
        ConvertToYaml,
        /// Show the focused editor's diff against the live object, or hide it
        /// (`editor::ShowDiff`).
        ShowDiff,
        /// Switch the focused editor's diff between side by side and inline
        /// (`editor::ToggleDiffLayout`).
        ToggleDiffLayout,
        /// Replace the focused editor's buffer with the live object
        /// (`editor::ReloadFromServer`).
        ReloadFromServer,
        /// Open the focused manifest editor's completion menu at the cursor
        /// (`editor::TriggerCompletion`).
        TriggerCompletion,
        /// Show the schema docs of the token at the focused manifest editor's cursor
        /// (`editor::ShowHover`).
        ShowHover,
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
