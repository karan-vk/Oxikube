//! `editor::NewManifest`, `editor::ToggleReadOnly`, `editor::ToggleSoftWrap` (E10-S04),
//! `resource::Edit`, `resource::ViewYamlInEditor`, `editor::ToggleManagedFields`,
//! `editor::ToggleSecretMask` (E10-S06), for JSON mode (E10-S10) `editor::ConvertToJson`,
//! `editor::ConvertToYaml`, `editor::UseJson` and `editor::UseYaml`, and the diff vs live commands
//! (E10-S07: `editor::ShowDiff`, `ToggleDiffLayout`, `ReloadFromServer`, `MergeFromServer`,
//! `KeepMine`, `TakeMine`, `TakeServer`, `AcceptMerge`) on the command bus, each with its MCP tool
//! stub (`app.editor_new_manifest`, `k8s.resource_edit`, ...).
//!
//! The keymap (through the GPUI actions of the same names), the toolbar, the palette and agents
//! all dispatch the bus command. Its handler queues an [`EditorRequest`] on the window's
//! [`EditorViewSink`]; the window's [`EditorViews`](super::EditorViews) applies it on the UI
//! thread. Nothing here changes a cluster: no `MutationGuard` tier (the editor's read-only is a
//! view state, not the cluster's read-only mode). The diff commands read the edited object and
//! change the buffer only.

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::{OxiError, OxiResult};

use crate::diff::MergeSide;
use crate::json::Language;
use crate::open::OpenMode;

/// The commands [`register_commands`] handles.
pub const EDITOR_COMMANDS: [CommandId; 19] = [
    CommandId::EDITOR_NEW_MANIFEST,
    CommandId::EDITOR_TOGGLE_MANAGED_FIELDS,
    CommandId::EDITOR_TOGGLE_READ_ONLY,
    CommandId::EDITOR_TOGGLE_SECRET_MASK,
    CommandId::EDITOR_TOGGLE_SOFT_WRAP,
    CommandId::RESOURCE_EDIT,
    CommandId::RESOURCE_VIEW_YAML_IN_EDITOR,
    CommandId::EDITOR_CONVERT_TO_JSON,
    CommandId::EDITOR_CONVERT_TO_YAML,
    CommandId::EDITOR_USE_JSON,
    CommandId::EDITOR_USE_YAML,
    CommandId::EDITOR_SHOW_DIFF,
    CommandId::EDITOR_TOGGLE_DIFF_LAYOUT,
    CommandId::EDITOR_RELOAD_FROM_SERVER,
    CommandId::EDITOR_MERGE_FROM_SERVER,
    CommandId::EDITOR_KEEP_MINE,
    CommandId::EDITOR_TAKE_MINE,
    CommandId::EDITOR_TAKE_SERVER,
    CommandId::EDITOR_ACCEPT_MERGE,
];

/// What the window's manifest editors should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorRequest {
    /// `editor::NewManifest`: an empty editor in `cluster`'s tab (the shown one when `None`).
    New {
        /// The cluster whose tab gets the editor.
        cluster: Option<ClusterId>,
    },
    /// `resource::Edit` (`mode` edit) or `resource::ViewYamlInEditor` (`mode` view): a tab titled
    /// `kind/name` in the object's cluster tab, reading the object fresh. The tab of the same
    /// object is reused.
    Open {
        /// The object.
        target: ResourceRef,
        /// Editable or read-only.
        mode: OpenMode,
    },
    /// `editor::ToggleManagedFields` on the focused editor, else the active pane's.
    ToggleManagedFields,
    /// `editor::ToggleSecretMask` on the focused editor, else the active pane's.
    ToggleSecretMask,
    /// `editor::ToggleReadOnly` on the focused editor, else the active pane's.
    ToggleReadOnly,
    /// `editor::ToggleSoftWrap` on the focused editor, else the active pane's.
    ToggleSoftWrap,
    /// `editor::ConvertToJson` / `editor::ConvertToYaml` on the focused editor, else the active
    /// pane's: replace its text with the same data in this language (one undo step).
    ConvertTo(Language),
    /// `editor::UseJson` / `editor::UseYaml` on the focused editor, else the active pane's: read
    /// the buffer as this language from now on.
    UseLanguage(Language),
    /// `editor::ShowDiff`: the diff panel of the focused editor (else the active pane's).
    ShowDiff,
    /// `editor::ToggleDiffLayout`: side by side or inline.
    ToggleDiffLayout,
    /// `editor::ReloadFromServer`: the live object into the buffer.
    ReloadFromServer,
    /// `editor::MergeFromServer`: the server's change merged into the buffer.
    MergeFromServer,
    /// `editor::KeepMine`: the buffer kept over the server's change.
    KeepMine,
    /// `editor::TakeMine` / `editor::TakeServer`: a side for one conflict of the merge.
    Take {
        /// The conflict, counting from 0.
        conflict: usize,
        /// The side.
        side: MergeSide,
    },
    /// `editor::AcceptMerge`: the settled merge into the buffer.
    AcceptMerge,
}

impl EditorRequest {
    /// The request `command` asks for; `None` for any other command.
    pub fn of(command: Command) -> Option<Self> {
        match command {
            Command::EditorNewManifest { cluster } => Some(EditorRequest::New { cluster }),
            Command::ResourceEdit { target } => Some(EditorRequest::Open {
                target,
                mode: OpenMode::Edit,
            }),
            Command::ResourceViewYamlInEditor { target } => Some(EditorRequest::Open {
                target,
                mode: OpenMode::View,
            }),
            Command::EditorToggleManagedFields => Some(EditorRequest::ToggleManagedFields),
            Command::EditorToggleSecretMask => Some(EditorRequest::ToggleSecretMask),
            Command::EditorToggleReadOnly => Some(EditorRequest::ToggleReadOnly),
            Command::EditorToggleSoftWrap => Some(EditorRequest::ToggleSoftWrap),
            Command::EditorConvertToJson => Some(EditorRequest::ConvertTo(Language::Json)),
            Command::EditorConvertToYaml => Some(EditorRequest::ConvertTo(Language::Yaml)),
            Command::EditorUseJson => Some(EditorRequest::UseLanguage(Language::Json)),
            Command::EditorUseYaml => Some(EditorRequest::UseLanguage(Language::Yaml)),
            Command::EditorShowDiff => Some(EditorRequest::ShowDiff),
            Command::EditorToggleDiffLayout => Some(EditorRequest::ToggleDiffLayout),
            Command::EditorReloadFromServer => Some(EditorRequest::ReloadFromServer),
            Command::EditorMergeFromServer => Some(EditorRequest::MergeFromServer),
            Command::EditorKeepMine => Some(EditorRequest::KeepMine),
            Command::EditorTakeMine { conflict } => Some(EditorRequest::Take {
                conflict: conflict as usize,
                side: MergeSide::Mine,
            }),
            Command::EditorTakeServer { conflict } => Some(EditorRequest::Take {
                conflict: conflict as usize,
                side: MergeSide::Server,
            }),
            Command::EditorAcceptMerge => Some(EditorRequest::AcceptMerge),
            _ => None,
        }
    }
}

/// A handle on a window's editor request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct EditorViewSink {
    tx: UnboundedSender<EditorRequest>,
}

impl EditorViewSink {
    /// A sink and the receiver the window's [`EditorViews`](super::EditorViews) drains.
    pub fn channel() -> (Self, UnboundedReceiver<EditorRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Queues `request`. Fails when the window is gone.
    pub fn send(&self, request: EditorRequest) -> OxiResult<()> {
        self.tx
            .unbounded_send(request)
            .map_err(|_| OxiError::internal("the window with the editors is gone"))
    }
}

/// Registers the [`EDITOR_COMMANDS`] on `registry` (each with its MCP tool stub). Call it from
/// the binary's command setup: `registry.install("oxikube_editor", |r| register_commands(r, sink))`.
///
/// # Errors
///
/// A [`RegisterError`] when an id is registered twice (a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: EditorViewSink,
) -> Result<(), RegisterError> {
    for id in EDITOR_COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let request = EditorRequest::of(command)
                    .ok_or_else(|| OxiError::validation("not a manifest editor command"))?;
                sink.send(request)?;
                Ok(CommandOutput::none())
            }
        })?;
    }
    Ok(())
}
