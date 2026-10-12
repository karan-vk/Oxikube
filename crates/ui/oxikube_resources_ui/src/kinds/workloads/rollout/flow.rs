//! [`SetImageFlow`]: from "Set image" on a workload to the open dialog.

use std::sync::Arc;

use gpui::{App, AppContext as _, WeakEntity, Window};
use oxikube_app::{SetImageRunner, WorkloadService};
use oxikube_domain::ids::ResourceRef;
use oxikube_workspace::Workspace;

use super::SetImageDialog;

/// Opens the set-image dialog for a workload in a cluster tab's workspace. Cheap to clone. See the
/// [module docs](super).
#[derive(Clone)]
pub struct SetImageFlow {
    service: Arc<WorkloadService>,
    runner: SetImageRunner,
    workspace: WeakEntity<Workspace>,
}

impl SetImageFlow {
    /// A flow that reads through `service`, sets through `runner` and asks in `workspace`.
    pub fn new(
        service: Arc<WorkloadService>,
        runner: SetImageRunner,
        workspace: WeakEntity<Workspace>,
    ) -> Self {
        Self {
            service,
            runner,
            workspace,
        }
    }

    /// Opens the dialog for `target` at once; it reads the workload's containers itself. Does
    /// nothing without a live workspace. Returns the dialog so a caller can drive it.
    pub fn begin(
        &self,
        target: ResourceRef,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui::Entity<SetImageDialog>> {
        let workspace = self.workspace.upgrade()?;
        let (service, runner, weak) = (
            self.service.clone(),
            self.runner.clone(),
            self.workspace.clone(),
        );
        Some(workspace.update(cx, |workspace, cx| {
            let dialog =
                cx.new(|cx| SetImageDialog::new(target, service, runner, weak, window, cx));
            workspace.show_modal(dialog.clone(), window, cx);
            dialog
        }))
    }
}
