//! [`BulkDialog`]: confirm (or edit), run and report a bulk action on a selection.
//!
//! The dialog is a modal of the cluster tab's workspace. It plans with
//! [`SelectionFlow::plan`](oxikube_app::SelectionFlow::plan), so what it asks is what the guard
//! will ask. Cancel sends nothing. Confirming runs the flow on the Tokio bridge: one command, one
//! guard check for the whole batch, and the dialog then lists what happened to each object, with
//! a retry for the rows that failed.

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, WeakEntity, Window,
};
use oxikube_app::{BulkReport, SelectionFlow, SelectionPlan};
use oxikube_domain::command::{Command, Propagation};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::selection::{LabelEdit, LabelField};
use oxikube_ui::input::InputState;
use oxikube_workspace::Workspace;
use oxikube_workspace::modal::{ModalPlacement, ModalView};

use super::editor::LabelEditor;
use super::kind::BulkKind;

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BulkStage {
    /// Asking. Nothing has been sent.
    Confirm,
    /// The command is running.
    Running,
    /// Every object has a result.
    Done,
}

/// The sample object the label editor previews against: the first selected row.
#[derive(Clone, Debug, Default)]
pub struct BulkSample {
    /// Its name.
    pub name: String,
    /// Its labels.
    pub labels: Vec<(String, String)>,
    /// Its annotations.
    pub annotations: Vec<(String, String)>,
}

/// The bulk confirmation, label editor and results. See the [module docs](super).
pub struct BulkDialog {
    pub(super) kind: BulkKind,
    pub(super) flow: SelectionFlow,
    pub(super) plan: SelectionPlan,
    pub(super) typed: Entity<InputState>,
    pub(super) stage: BulkStage,
    pub(super) report: Option<BulkReport>,
    /// The label editor, for `selection::Label`.
    pub(super) editor: Option<LabelEditor>,
    /// Why nothing ran (the phrase did not match, the batch was refused), shown under the form.
    pub(super) error: Option<SharedString>,
    pub(super) workspace: WeakEntity<Workspace>,
    pub(super) focus: FocusHandle,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for BulkDialog {}

impl Focusable for BulkDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for BulkDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Escape and a click outside cancel while asking; while the batch runs the dialog stays (it
    /// finishes either way, and the results are what the user wants to see).
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage != BulkStage::Running
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage != BulkStage::Running
    }
}

impl BulkDialog {
    /// A dialog for `kind` over the objects of `plan` (from [`BulkFlow::begin`](super::BulkFlow::begin),
    /// which plans the kind's starting command). `sample` is the object the label editor previews
    /// (ignored by the other kinds).
    pub fn new(
        kind: BulkKind,
        flow: SelectionFlow,
        plan: SelectionPlan,
        sample: BulkSample,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let typed = cx
            .new(|cx| InputState::new(window, cx).placeholder("Type the cluster name to confirm"));
        let subscriptions = vec![cx.subscribe_in(&typed, window, Self::on_typed)];
        let editor = (kind == BulkKind::Label)
            .then(|| LabelEditor::new(sample.name, sample.labels, sample.annotations, window, cx));
        Self {
            kind,
            flow,
            plan,
            typed,
            stage: BulkStage::Confirm,
            report: None,
            editor,
            error: None,
            workspace,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The command of `kind` over `targets`. A label command without `edits` is the editor's
    /// starting point: planned for its tier and counts, never run.
    pub(super) fn command_for(
        kind: BulkKind,
        targets: Vec<ResourceRef>,
        propagation: Propagation,
        label: Option<(LabelField, Vec<LabelEdit>)>,
    ) -> Command {
        match kind {
            BulkKind::Delete => Command::SelectionDelete {
                targets,
                propagation,
            },
            BulkKind::Restart => Command::SelectionRestart { targets },
            BulkKind::Label => {
                let (field, edits) = label.unwrap_or_default();
                Command::SelectionLabel {
                    targets,
                    field,
                    edits,
                }
            }
        }
    }

    /// Focuses the field the user types in: the typed phrase, the editor's first key, or the
    /// dialog itself.
    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = &self.editor {
            if let Some(row) = editor.rows.first() {
                row.key.update(cx, |input, cx| input.focus(window, cx));
                return;
            }
        }
        if self.plan.phrase().is_some() {
            self.typed.update(cx, |input, cx| input.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
    }

    /// Which bulk action this is.
    pub fn kind(&self) -> BulkKind {
        self.kind
    }

    /// Where the dialog is.
    pub fn stage(&self) -> BulkStage {
        self.stage
    }

    /// What the dialog will ask and send.
    pub fn plan(&self) -> &SelectionPlan {
        &self.plan
    }

    /// The results, once the command has run.
    pub fn report(&self) -> Option<&BulkReport> {
        self.report.as_ref()
    }

    /// Shows `report` as the finished results (tests of the results list).
    #[cfg(test)]
    pub(super) fn set_report_for_test(&mut self, report: BulkReport) {
        self.report = Some(report);
        self.stage = BulkStage::Done;
    }

    /// The label editor, for `selection::Label`.
    pub fn editor(&self) -> Option<&LabelEditor> {
        self.editor.as_ref()
    }

    /// The label editor, to drive it as the user would.
    pub fn editor_mut(&mut self) -> Option<&mut LabelEditor> {
        self.editor.as_mut()
    }

    /// The message under the form, if the batch was refused.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// What has been typed, trimmed.
    pub fn typed_text(&self, cx: &App) -> String {
        self.typed.read(cx).value().trim().to_owned()
    }

    /// Types `text` into the confirmation field, as the user would.
    pub fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.typed
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    /// Chooses how dependents are handled (Delete) and asks again: a cascading choice raises what
    /// has to be typed.
    pub fn set_propagation(&mut self, propagation: Propagation, cx: &mut Context<Self>) {
        let Command::SelectionDelete {
            propagation: current,
            targets,
        } = self.plan.command()
        else {
            return;
        };
        if self.stage != BulkStage::Confirm || propagation == *current {
            return;
        }
        let command = Command::SelectionDelete {
            targets: targets.clone(),
            propagation,
        };
        if let Ok(plan) = self.flow.plan(command) {
            self.plan = plan;
            self.error = None;
            cx.notify();
        }
    }

    /// The propagation a Delete will use.
    pub fn propagation(&self) -> Propagation {
        match self.plan.command() {
            Command::SelectionDelete { propagation, .. } => *propagation,
            _ => Propagation::Background,
        }
    }

    /// Switches the editor between labels and annotations.
    pub fn set_field(&mut self, field: LabelField, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.as_mut()
            && self.stage == BulkStage::Confirm
            && editor.field != field
        {
            editor.field = field;
            self.error = None;
            cx.notify();
        }
    }

    /// Whether the button can be pressed: asking, the phrase typed when one is needed, and for
    /// the editor, valid edits.
    pub fn can_confirm(&self, cx: &App) -> bool {
        self.stage == BulkStage::Confirm
            && self
                .plan
                .phrase()
                .is_none_or(|phrase| self.typed_text(cx) == phrase)
            && self
                .editor
                .as_ref()
                .is_none_or(|editor| editor.is_ready(cx))
    }

    /// Cancels while asking, closes once finished: nothing is sent, so there is nothing to undo.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage != BulkStage::Running {
            cx.emit(DismissEvent);
        }
    }
}
