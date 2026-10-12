//! Running the bulk dialog: confirm, retry of the failed rows, the run on the Tokio bridge and what
//! comes back.

use gpui::{Context, DismissEvent, Entity, Window};
use oxikube_app::{BulkReport, SelectionError, SelectionPlan};
use oxikube_domain::command::Propagation;
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::Toast;

use super::dialog::{BulkDialog, BulkStage};
use super::kind::BulkKind;

impl BulkDialog {
    /// Runs the action. Does nothing while the phrase is not typed or the edits are not valid.
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        if !self.can_confirm(cx) {
            return;
        }
        let plan = match &self.editor {
            Some(editor) => {
                // The edits are final now: the plan is the guard's answer for exactly them.
                let command = Self::command_for(
                    BulkKind::Label,
                    self.plan.targets().to_vec(),
                    Propagation::Background,
                    Some((editor.field, editor.edits(cx))),
                );
                match self.flow.plan(command) {
                    Ok(plan) => plan,
                    Err(error) => {
                        self.error = Some(error.to_string().into());
                        cx.notify();
                        return;
                    }
                }
            }
            None => self.plan.clone(),
        };
        self.plan = plan.clone();
        self.run(plan, false, cx);
    }

    /// Runs the same action again over only the rows that failed. Does nothing when none did.
    pub fn retry_failed(&mut self, cx: &mut Context<Self>) {
        if self.stage != BulkStage::Done {
            return;
        }
        let Some(report) = &self.report else { return };
        let failed = report.failed_targets();
        if failed.is_empty() {
            return;
        }
        match self.flow.plan_again(&self.plan, failed) {
            Ok(plan) => self.run(plan, true, cx),
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    fn run(&mut self, plan: SelectionPlan, retry: bool, cx: &mut Context<Self>) {
        let typed = plan.phrase().map(|_| self.typed_text(cx));
        self.stage = BulkStage::Running;
        self.error = None;
        cx.notify();
        let flow = self.flow.clone();
        let workspace = self.workspace.clone();
        // Detached: the batch must finish even if the dialog goes away meanwhile (every object is
        // already audited as it runs); the task ends itself and clears nothing.
        cx.spawn(async move |this, cx| {
            let run = spawn_kube(cx, async move { flow.run(&plan, typed.as_deref()).await });
            let result = match run.await {
                Ok(Ok(report)) => Ok(report),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(SelectionError::Refused(oxikube_domain::OxiError::internal(
                    error.to_string(),
                ))),
            };
            let toast = match &result {
                Ok(report) => summary_toast(report),
                Err(error) => Toast::error(error.to_string()),
            };
            let delivered = this.update(cx, |this, cx| this.finished(result, retry, cx));
            if delivered.is_err() {
                // The dialog is gone: say how it went where the user still looks.
                workspace
                    .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn finished(
        &mut self,
        result: Result<BulkReport, SelectionError>,
        retry: bool,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(report) => {
                let report = match (&self.report, retry) {
                    (Some(first), true) => first.merged_with(&report),
                    _ => report,
                };
                self.stage = BulkStage::Done;
                let toast = summary_toast(&report);
                let single_ok = report.rows.len() == 1 && report.failed() == 0;
                self.report = Some(report);
                if single_ok {
                    // A result list for one object that went well is friction: toast and close.
                    self.show_toast(toast, cx);
                    cx.emit(DismissEvent);
                } else {
                    cx.notify();
                }
            }
            Err(error) => {
                // Nothing ran (the typed text was wrong, or the batch was refused as a whole):
                // back to the form, or the results when there were some, with the reason.
                self.stage = if self.report.is_some() {
                    BulkStage::Done
                } else {
                    BulkStage::Confirm
                };
                self.error = Some(match &error {
                    SelectionError::Refused(error) => error.message().to_owned().into(),
                    other => other.to_string().into(),
                });
                cx.notify();
            }
        }
    }

    fn show_toast(&self, toast: Toast, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
            .ok();
    }

    /// A field of the dialog changed (the typed phrase or an editor input): the button and the
    /// preview follow; Enter confirms.
    pub(super) fn on_typed(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.error = None;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.confirm(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}

/// The toast for a finished batch.
fn summary_toast(report: &BulkReport) -> Toast {
    if report.failed() == 0 {
        Toast::success(report.summary())
    } else {
        Toast::warning(report.summary())
    }
}
