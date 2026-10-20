//! [`StartFlow`]: from a Forward button, `shift-f` or the palette to the open dialog.

use gpui::{App, AppContext as _, WeakEntity, Window};
use oxikube_app::{ClusterSessionManager, ForwardStarter};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::portforward::ForwardPort;
use oxikube_workspace::{Toast, Workspace};

use super::form::StartForm;
use super::ports::PortChoice;
use super::view::StartDialog;

/// What to forward, as far as the caller knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardRequest {
    /// The Pod or Service.
    pub target: ResourceRef,
    /// The port that was clicked; the dialog starts on it.
    pub port: Option<ForwardPort>,
    /// The target's ports, when the caller has the object (a detail panel); empty makes the
    /// dialog read them.
    pub ports: Vec<PortChoice>,
}

impl ForwardRequest {
    /// A request for `target` with nothing else known (the row's `shift-f`, the palette).
    pub fn new(target: ResourceRef) -> Self {
        Self {
            target,
            port: None,
            ports: Vec::new(),
        }
    }

    /// Starts on `port`.
    #[must_use]
    pub fn with_port(mut self, port: ForwardPort) -> Self {
        self.port = Some(port);
        self
    }

    /// Offers `ports`, already read.
    #[must_use]
    pub fn with_ports(mut self, ports: Vec<PortChoice>) -> Self {
        self.ports = ports;
        self
    }
}

/// Opens the start dialog for a Pod or a Service in a cluster tab's workspace. Cheap to clone.
/// See the [module docs](super).
#[derive(Clone)]
pub struct StartFlow {
    starter: ForwardStarter,
    sessions: ClusterSessionManager,
    workspace: WeakEntity<Workspace>,
}

impl StartFlow {
    /// A flow that starts through `starter`, reads ports through `sessions` and asks in
    /// `workspace`.
    pub fn new(
        starter: ForwardStarter,
        sessions: ClusterSessionManager,
        workspace: WeakEntity<Workspace>,
    ) -> Self {
        Self {
            starter,
            sessions,
            workspace,
        }
    }

    /// Why a Forward button for `cluster` is disabled now (read-only, with
    /// `portforward.allowed_in_readonly` off): the tooltip. `None` when it is enabled.
    pub fn blocked(&self, cluster: &ClusterId) -> Option<String> {
        self.starter.blocked(cluster)
    }

    /// Opens the dialog for `request` at once (no I/O first: a request without ports makes the
    /// dialog read them afterwards). A blocked cluster gets a toast saying why instead. Returns
    /// the dialog so a caller can drive it.
    pub fn begin(
        &self,
        request: ForwardRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui::Entity<StartDialog>> {
        let workspace = self.workspace.upgrade()?;
        if let Some(reason) = self.blocked(&request.target.cluster) {
            workspace.update(cx, |workspace, cx| {
                workspace.show_toast(
                    Toast::warning(reason).key("action-disabled:portforward::Start"),
                    cx,
                );
            });
            return None;
        }
        // A pod is read for its annotations too, when they are on.
        let wants_annotations = &*request.target.gvk.kind == "Pod" && self.starter.annotations();
        let reader = (request.ports.is_empty() || wants_annotations)
            .then(|| {
                self.sessions
                    .get(&request.target.cluster)
                    .and_then(|session| session.resources())
            })
            .flatten();
        let (starter, host) = (self.starter.clone(), self.workspace.clone());
        let ForwardRequest {
            target,
            port,
            ports,
        } = request;
        Some(workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| {
                let mut dialog = StartDialog::new(
                    StartForm::new(target, ports),
                    port,
                    starter,
                    host,
                    window,
                    cx,
                );
                if let Some(reader) = reader {
                    dialog.load_ports(reader, window, cx);
                }
                dialog
            });
            workspace.show_modal(dialog.clone(), window, cx);
            dialog.update(cx, |dialog, cx| dialog.focus_local(window, cx));
            dialog
        }))
    }
}
