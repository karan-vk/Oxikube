//! `ExecFlow::begin_debug`: from "Debug" on a pod to the open dialog.

use gpui::{App, AppContext as _, Context, Task, Window};
use oxikube_app::{DebugDefaults, DebugRunner};
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ResourceRef;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::Toast;

use super::DebugDialog;
use crate::exec::ExecFlow;

impl ExecFlow {
    /// Lets this flow add debug containers: "Debug" runs `pod::Debug` through `runner`. Without it
    /// the flow only opens shells and attaches.
    #[must_use]
    pub fn with_debug(mut self, runner: DebugRunner) -> Self {
        self.debug = Some(runner);
        self
    }

    /// Whether this flow can add debug containers.
    pub fn can_debug(&self) -> bool {
        self.debug.is_some()
    }

    /// Starts "Debug" on `target`: reads the pod for the dialog's defaults, then opens the dialog
    /// in the workspace. Returns the task that does both; the caller keeps it (dropping it cancels
    /// the read).
    ///
    /// A pod that cannot be read (not found, no `get`) is a toast with the reason: the dialog
    /// needs the pod's containers to offer a target.
    pub fn begin_debug<V: 'static>(
        &self,
        target: ResourceRef,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Task<()> {
        let service = self.service.clone();
        let loaded = spawn_kube(cx, async move { service.debug_defaults(&target).await });
        let flow = self.clone();
        cx.spawn_in(window, async move |_, cx| {
            let defaults = loaded.await.unwrap_or_else(|error| Err(error.into()));
            cx.update(|window, cx| flow.show_debug(defaults, window, cx))
                .ok();
        })
    }

    /// Opens the dialog for `defaults`, or says why it cannot.
    fn show_debug(&self, defaults: OxiResult<DebugDefaults>, window: &mut Window, cx: &mut App) {
        let (Some(runner), Some(workspace)) = (self.debug.clone(), self.workspace.upgrade()) else {
            return;
        };
        match defaults {
            Ok(defaults) => {
                let weak = self.workspace.clone();
                workspace.update(cx, |workspace, cx| {
                    let dialog = cx.new(|cx| DebugDialog::new(defaults, runner, weak, window, cx));
                    workspace.show_modal(dialog.clone(), window, cx);
                    dialog.update(cx, |dialog, cx| dialog.focus_image(window, cx));
                });
            }
            Err(error) => {
                let toast = Toast::error(format!("Cannot debug the pod: {}", error.message()));
                workspace.update(cx, |workspace, cx| workspace.show_toast(toast, cx));
            }
        }
    }
}
