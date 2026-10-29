//! Copy to… from a detail (E20-S08): whether the view offers it, and the flow that opens the
//! pickers. The command palette (a focused detail's own command) goes through here.

use gpui::{Context, Window};

use super::view::DetailView;
use crate::copy::CopyFlow;

impl DetailView {
    /// Lets the palette's "Copy to…" open the copy flow for this object.
    pub fn set_copy(&mut self, flow: Option<CopyFlow>) {
        self.copy = flow;
    }

    /// What the view does about copying: `None` when it offers none (no flow wired), else `Some(None)`.
    /// The source's read-only flag never blocks a copy: only the target is written.
    pub fn copy_state(&self) -> Option<Option<String>> {
        self.copy.as_ref().map(|_| None)
    }

    /// Opens the cluster picker for the object. Does nothing when copying is not on offer.
    pub fn open_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(flow) = self.copy.clone() {
            flow.begin(self.target.clone(), window, cx);
        }
    }
}
