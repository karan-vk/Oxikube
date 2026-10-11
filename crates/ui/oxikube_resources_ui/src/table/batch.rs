//! The CronJob and Job keys (E12-S08): `t` triggers a CronJob, `s` suspends or resumes a CronJob
//! or a Job.
//!
//! Both are view actions that stand for commands (`cronjob::Trigger`, `cronjob::Suspend` /
//! `cronjob::Resume`, `job::Suspend` / `job::Resume`), so the key, the context menu, the palette
//! and an agent run one behaviour; the table never calls a port. The default bindings are
//! kind-scoped sections in the per-OS keymap files (`ResourceTable && !Editing && kind ==
//! CronJob`), so `s` stays a shell on pods and nodes. Both act on the cursor row of a selection and
//! say so.

use gpui::{Context, Window};
use oxikube_domain::command::CommandId;
use oxikube_domain::{CronJobSummary, JobSummary};

use super::actions::{ToggleSuspendSelected, TriggerSelected};
use super::view::ResourceTable;
use crate::kinds::job;

impl ResourceTable {
    /// `t`: the `cronjob::Trigger` row action, on the cursor row.
    pub(super) fn on_trigger(
        &mut self,
        _: &TriggerSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::CRONJOB_TRIGGER, targets, window, cx);
    }

    /// `s`: suspends the cursor row's CronJob or Job, or resumes it when it is suspended.
    pub(super) fn on_toggle_suspend(
        &mut self,
        _: &ToggleSuspendSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        let suspended = self.cursor_suspended(cx);
        let command = match (job::is_job(&self.kind), suspended) {
            (true, true) => CommandId::JOB_RESUME,
            (true, false) => CommandId::JOB_SUSPEND,
            (false, true) => CommandId::CRONJOB_RESUME,
            (false, false) => CommandId::CRONJOB_SUSPEND,
        };
        self.run_action(command, targets, window, cx);
    }

    /// Whether the cursor row's object is suspended (`spec.suspend`); `false` for no row or a
    /// kind that cannot be.
    fn cursor_suspended(&self, cx: &gpui::App) -> bool {
        let Some(row) = self.cursor_row(cx) else {
            return false;
        };
        self.read_rows(cx, |rows| {
            let Some(resource) = rows.row(row).and_then(|object| object.resource()) else {
                return false;
            };
            if job::is_job(&self.kind) {
                JobSummary::from_resource(resource).is_ok_and(|summary| summary.suspend)
            } else {
                CronJobSummary::from_resource(resource).is_ok_and(|summary| summary.suspend)
            }
        })
    }
}
