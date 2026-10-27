//! `role::OpenDialog`: open the rule dialog in a cluster's tab.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, AppContext as _, Task, WeakEntity, Window};
use oxikube_app::ClusterSessionManager;
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::rbac::RoleKind;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::{ClusterTabs, CommandDispatcher};

use super::dialog::{DialogMode, RoleDialog, RoleDialogDeps};

/// One `role::OpenDialog`, as the window applies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleRequest {
    /// The cluster whose tab gets the dialog; the shown one when `None`.
    pub cluster: Option<ClusterId>,
    /// What to open.
    pub mode: RequestMode,
}

/// What a [`RoleRequest`] opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestMode {
    /// A new Role or ClusterRole.
    New(RoleKind),
    /// The rules of this Role or ClusterRole.
    Edit(ResourceRef),
}

impl RoleRequest {
    /// The request `command` asks for; `None` for any other command or a target that is not a
    /// Role or ClusterRole.
    pub fn of(command: &Command) -> Option<Self> {
        let Command::RoleOpenDialog {
            cluster,
            cluster_role,
            target,
        } = command
        else {
            return None;
        };
        let mode = match target {
            Some(target) if matches!(&*target.gvk.kind, "Role" | "ClusterRole") => {
                RequestMode::Edit(target.clone())
            }
            Some(_) => return None,
            None if *cluster_role => RequestMode::New(RoleKind::ClusterRole),
            None => RequestMode::New(RoleKind::Role),
        };
        Some(Self {
            cluster: cluster.clone(),
            mode,
        })
    }
}

/// A handle on the window's dialog-request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct RoleRequestSink {
    tx: UnboundedSender<RoleRequest>,
}

impl RoleRequestSink {
    /// A sink and the receiver [`RoleViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<RoleRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to open the dialog. Fails when the window is gone.
    pub fn send(&self, request: RoleRequest) -> OxiResult<()> {
        self.tx
            .unbounded_send(request)
            .map_err(|_| OxiError::internal("the window with the cluster tabs is gone"))
    }
}

/// Registers `role::OpenDialog` (tool stub `k8s.role_open_dialog`) on `registry`. The binary
/// installs it as `oxikube_rbac_ui::role`.
///
/// # Errors
///
/// A [`RegisterError`] when the id is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: RoleRequestSink,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::ROLE_OPEN_DIALOG)
        .ok_or(RegisterError::Undeclared(CommandId::ROLE_OPEN_DIALOG))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            let request = RoleRequest::of(&command).ok_or_else(|| {
                OxiError::validation("role::OpenDialog opens a Role or a ClusterRole")
            })?;
            sink.send(request)?;
            Ok(CommandOutput::none())
        }
    })
}

/// What opening the dialog needs.
#[derive(Clone)]
pub struct RoleViewsDeps {
    /// The sessions the dialog reads through.
    pub sessions: ClusterSessionManager,
    /// Where the dialog's save goes (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The window's cluster tabs.
    pub tabs: WeakEntity<ClusterTabs>,
}

/// The window's rule dialogs.
pub struct RoleViews;

impl RoleViews {
    /// Starts applying the requests of `requests` on the UI thread. Keep the returned task for the
    /// life of the window (dropping it stops it).
    pub fn start(
        deps: RoleViewsDeps,
        mut requests: UnboundedReceiver<RoleRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            while let Some(request) = requests.next().await {
                match cx.update(|window, cx| Self::open(&deps, &request, window, cx)) {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::debug!("role::OpenDialog: no cluster tab is open for the dialog");
                    }
                    Err(_) => break,
                }
            }
        })
    }

    /// Shows the dialog in the request's cluster tab (the active one without a cluster) and brings
    /// the tab forward. `false` when there is no such tab.
    pub fn open(
        deps: &RoleViewsDeps,
        request: &RoleRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(tabs) = deps.tabs.upgrade() else {
            return false;
        };
        let from_target = match &request.mode {
            RequestMode::Edit(target) => Some(target.cluster.clone()),
            RequestMode::New(_) => None,
        };
        let Some(cluster) = request
            .cluster
            .clone()
            .or(from_target)
            .or_else(|| tabs.read(cx).active().cloned())
        else {
            return false;
        };
        let Some(workspace) = tabs
            .read(cx)
            .tab(&cluster)
            .map(|tab| tab.read(cx).workspace().clone())
        else {
            return false;
        };
        let mode = match &request.mode {
            RequestMode::Edit(target) => DialogMode::Edit(target.clone()),
            RequestMode::New(kind) => DialogMode::New {
                kind: *kind,
                namespace: deps
                    .sessions
                    .get(&cluster)
                    .and_then(|session| {
                        session
                            .namespace_selection()
                            .names()
                            .next()
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| "default".to_owned()),
            },
        };
        let dialog_deps = RoleDialogDeps {
            sessions: deps.sessions.clone(),
            dispatcher: deps.dispatcher.clone(),
            cluster: cluster.clone(),
        };
        tabs.update(cx, |tabs, cx| {
            tabs.activate(&cluster, window, cx);
        });
        workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| RoleDialog::new(dialog_deps, mode, window, cx));
            workspace.show_modal(dialog, window, cx);
        });
        true
    }
}
