//! Rollback from a detail (E12-S06): whether the view offers it, and the flow that opens the
//! dialog. The workload panel's button and the command palette (a focused detail's own command)
//! both go through here.

use gpui::{Context, Window};
use oxikube_app::ActionContext;
use oxikube_domain::command::{self, CommandId};

use super::view::DetailView;
use crate::kinds::can_rollback;

impl DetailView {
    /// What the view does about rolling back: `None` when it offers none (no flow wired, a kind
    /// with no history, no session), else why it is blocked now, if it is (a read-only cluster).
    pub fn rollback_state(&self) -> Option<Option<String>> {
        self.deps.rollback.as_ref()?;
        if !can_rollback(&self.target.gvk) {
            return None;
        }
        let session = self.deps.sessions.get(&self.target.cluster)?;
        let meta = command::lookup(CommandId::WORKLOAD_ROLLBACK)?;
        Some(
            ActionContext::of(&session)
                .state_of(meta)
                .reason()
                .map(|reason| reason.to_string()),
        )
    }

    /// Opens the rollback dialog for the object. Does nothing when rolling back is not on offer or
    /// is blocked (the button says why).
    pub fn open_rollback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rollback_state() != Some(None) {
            return;
        }
        if let Some(flow) = self.deps.rollback.clone() {
            flow.begin(self.target.clone(), window, cx);
        }
    }
}
