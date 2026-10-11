//! Applying the buffer to its cluster (E10-S08): `resource::Apply` on the focused editor.
//!
//! ```text
//! cmd-s / palette / toolbar ─> the buffer's own checks (off the UI thread): errors refuse
//!   ─> ApplyService::preview (server dry run, on the Kubernetes runtime)
//!        ├─ read-only ─────────> notice, nothing sent (the guard audits the refusal)
//!        ├─ invalid ───────────> the server's causes put on their fields, notice
//!        ├─ conflicts ─────────> ConflictDialog: managers, fields, times; Force previews again
//!        └─ preview ───────────> ApplyPreviewDialog: the diff the server would make
//!                                   └─ Apply ─> ApplyService::apply ─> toast, the buffer clean
//! ```
//!
//! | Module | What |
//! |---|---|
//! | `flow` | the steps above on [`ManifestEditor`]: [`begin_apply`](ManifestEditor::begin_apply), what each answer does to the editor |
//! | `preview` | [`ApplyPreviewDialog`]: the dry run's diff (virtualised), the fallback or force it will use, Apply / Cancel |
//! | `batch` | a buffer of several documents (E10-S09): every document's dry run, one combined preview ([`BatchPreviewDialog`]), applied in buffer order, the per-document results panel |
//! | `conflicts` | [`ConflictDialog`]: the fields other managers own, Force / Cancel |
//!
//! The editor never writes to a cluster: every request is a `resource::Apply` dispatch of
//! `oxikube_app::ApplyService` through the `MutationGuard`, run on the Kubernetes runtime
//! (`oxikube_runtime::spawn_kube`). The preview is the confirmation. Nothing here logs the
//! buffer; a Secret's values reach the dialogs only as fingerprints.

mod batch;
mod conflicts;
mod flow;
mod preview;

use gpui::{
    AnyElement, App, Context, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    WeakEntity, Window, actions, div,
};
use oxikube_app::{ApplyService, CommandTarget};
use oxikube_domain::command::{CommandId, ViewContext};
use oxikube_domain::ids::ResourceRef;
use oxikube_keymap::contexts;
use oxikube_ui::Sizable as _;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::Disableable as _;
use oxikube_ui::tooltip::tooltip_for_action;
use oxikube_workspace::Workspace;
use oxikube_workspace::command_surface::CommandSurface;

pub use batch::{BatchPreviewDialog, BatchSession};
pub use conflicts::ConflictDialog;
pub use preview::ApplyPreviewDialog;

use super::manifest_editor::ManifestEditor;

actions!(
    resource,
    [
        /// Apply the focused manifest editor's buffer to its cluster (`resource::Apply`): a
        /// server dry run, its diff to confirm, then a server-side apply.
        Apply,
    ]
);

/// Where an editor's apply stands. One at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApplyStage {
    /// Nothing running.
    #[default]
    Idle,
    /// The checks and the server's dry run are running.
    DryRun,
    /// The preview (or the conflict list) is open, waiting for the user.
    Reviewing,
    /// The apply was sent.
    Applying,
}

/// An editor's way to its cluster: the service, the workspace its dialogs open in, the stage.
#[derive(Default)]
pub(crate) struct ApplyLink {
    pub(super) service: Option<ApplyService>,
    pub(super) workspace: Option<WeakEntity<Workspace>>,
    pub(super) stage: ApplyStage,
    /// The step running; replacing it cancels it. Never cleared from inside (see `stage`).
    pub(super) task: Option<Task<()>>,
    /// The open preview's (or conflict list's) release: it ends `Reviewing` when the dialog goes
    /// without being answered, e.g. replaced by another modal.
    pub(super) review: Option<Subscription>,
    /// The results of the last multi-document apply, shown under the buffer (E10-S09).
    pub(super) results: Option<batch::ResultsPanel>,
}

impl ManifestEditor {
    /// Lets this editor apply its buffer through `service`, with its dialogs in `workspace` (the
    /// cluster tab's). Without it the editor edits text only.
    pub fn set_apply(&mut self, service: ApplyService, workspace: WeakEntity<Workspace>) {
        self.apply.service = Some(service);
        self.apply.workspace = Some(workspace);
    }

    /// Whether the editor can apply its buffer (it was given an [`ApplyService`] and a cluster).
    pub fn can_apply(&self) -> bool {
        self.apply.service.is_some() && self.cluster().is_some()
    }

    /// Where the apply stands.
    pub fn apply_stage(&self) -> ApplyStage {
        self.apply.stage
    }

    /// The toolbar's Apply button, when the editor can apply. Its tooltip names the key.
    pub(in crate::view) fn apply_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.can_apply() {
            return None;
        }
        let (label, busy) = match self.apply.stage {
            ApplyStage::Idle => ("Apply", false),
            ApplyStage::DryRun => ("Dry run…", true),
            ApplyStage::Reviewing => ("Apply", true),
            ApplyStage::Applying => ("Applying…", true),
        };
        let title = self.title.clone();
        Some(
            div()
                .id("manifest-apply")
                .flex_none()
                .debug_selector(move || format!("manifest-apply-{title}"))
                .tooltip(tooltip_for_action(
                    "Dry run, review the diff, apply",
                    &Apply,
                    Some(contexts::MANIFEST_EDITOR),
                ))
                .child(
                    Button::new("manifest-apply")
                        .label(label)
                        .primary()
                        .xsmall()
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, window, cx| {
                            window.focus(&view.editor.focus_handle(cx), cx);
                            view.begin_apply(window, cx);
                        })),
                )
                .into_any_element(),
        )
    }
}

/// The palette reads the editor as a command surface: it acts on the editor's cluster, and
/// `resource::Apply` (which needs the buffer) runs through the editor's own flow.
impl CommandSurface for ManifestEditor {
    fn view_context(&self) -> ViewContext {
        ViewContext::Editor
    }

    fn command_target(&self, _: &App) -> CommandTarget {
        match self.cluster() {
            Some(cluster) => CommandTarget::none().in_cluster(cluster.clone()),
            None => CommandTarget::none(),
        }
    }

    fn own_commands(&self, _: &App) -> Vec<CommandId> {
        vec![CommandId::RESOURCE_APPLY]
    }

    fn run_command(
        &mut self,
        command: CommandId,
        _: Vec<ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if command != CommandId::RESOURCE_APPLY {
            return false;
        }
        self.begin_apply(window, cx);
        true
    }
}
