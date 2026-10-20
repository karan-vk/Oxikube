//! Forward from a detail (E15-S04): whether the view offers it and the flow that opens the start
//! dialog. The Pod panel's container ports and the Service panel's ports go through here.

use gpui::{Context, Window};
use oxikube_portforward_ui::ForwardRequest;

use super::view::DetailView;

impl DetailView {
    /// What the view does about forwarding: `None` when it offers none (no flow wired), else why
    /// it is blocked now, if it is (a read-only cluster with `portforward.allowed_in_readonly`
    /// off).
    pub fn forward_state(&self) -> Option<Option<String>> {
        let flow = self.deps.forward.as_ref()?;
        Some(flow.blocked(&self.target.cluster))
    }

    /// Opens the start dialog for the object with `request`'s port and ports. Does nothing when
    /// forwarding is not on offer; a blocked cluster says why in a toast.
    pub fn open_forward(
        &mut self,
        request: ForwardRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(flow) = self.deps.forward.clone() {
            flow.begin(request, window, cx);
        }
    }
}
