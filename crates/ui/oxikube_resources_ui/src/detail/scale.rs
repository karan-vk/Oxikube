//! Scale from a detail (E12-S04): whether the view offers it, and the flow that opens the dialog.
//! The workload panel's button and the command palette (a focused detail's own command) both go
//! through here.

use gpui::{Context, Window};
use oxikube_app::ActionContext;
use oxikube_domain::command::{self, CommandId};

use super::view::DetailView;

impl DetailView {
    /// What the view does about scaling: `None` when it offers none (no flow wired, a kind that
    /// cannot be scaled, no session), else why it is blocked now, if it is (a read-only cluster).
    pub fn scale_state(&self) -> Option<Option<String>> {
        self.deps.scale.as_ref()?;
        if !oxikube_app::is_scalable(&self.target.gvk) {
            return None;
        }
        let session = self.deps.sessions.get(&self.target.cluster)?;
        let meta = command::lookup(CommandId::WORKLOAD_SCALE)?;
        Some(
            ActionContext::of(&session)
                .state_of(meta)
                .reason()
                .map(|reason| reason.to_string()),
        )
    }

    /// Opens the scale dialog for the object. Does nothing when scaling is not on offer or is
    /// blocked (the button says why).
    pub fn open_scale(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scale_state() != Some(None) {
            return;
        }
        if let Some(flow) = self.deps.scale.clone() {
            flow.begin(self.target.clone(), window, cx);
        }
    }
}
