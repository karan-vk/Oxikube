//! [`RollbackFlow`]: from "Rollback" on a workload to the open dialog.

use std::sync::Arc;

use gpui::{App, AppContext as _, WeakEntity, Window};
use oxikube_app::{RollbackRunner, RolloutService};
use oxikube_domain::ids::ResourceRef;
use oxikube_workspace::Workspace;

use super::RollbackDialog;

/// Opens the rollback dialog for a workload in a cluster tab's workspace. Cheap to clone. See the
/// [module docs](super).
#[derive(Clone)]
pub struct RollbackFlow {
    service: Arc<RolloutService>,
    runner: RollbackRunner,
    workspace: WeakEntity<Workspace>,
}

impl RollbackFlow {
    /// A flow that reads through `service`, rolls back through `runner` and asks in `workspace`.
    pub fn new(
        service: Arc<RolloutService>,
        runner: RollbackRunner,
        workspace: WeakEntity<Workspace>,
    ) -> Self {
        Self {
            service,
            runner,
            workspace,
        }
    }

    /// Opens the dialog for `target` at once; it reads the history itself. Does nothing without a
    /// live workspace. Returns the dialog so a caller can drive it.
    pub fn begin(
        &self,
        target: ResourceRef,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui::Entity<RollbackDialog>> {
        let workspace = self.workspace.upgrade()?;
        let (service, runner, weak) = (
            self.service.clone(),
            self.runner.clone(),
            self.workspace.clone(),
        );
        Some(workspace.update(cx, |workspace, cx| {
            let dialog =
                cx.new(|cx| RollbackDialog::new(target, service, runner, weak, window, cx));
            workspace.show_modal(dialog.clone(), window, cx);
            dialog.update(cx, |dialog, cx| dialog.focus(window, cx));
            dialog
        }))
    }
}
