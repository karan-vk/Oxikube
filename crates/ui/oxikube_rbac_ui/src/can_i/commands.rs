//! `rbac::CanI` on the bus and the window's dialogs.
//!
//! The bus handler ([`register_commands`]) queues the request on a [`CanIRequestSink`]; the
//! window's [`CanIViews`] drains the queue on the UI thread, brings the cluster's tab forward and
//! shows the [`CanIDialog`] in its workspace, prefilled from the selected object. Nothing here asks
//! the cluster: the dialog does, when the user presses Ask.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, AppContext as _, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_app::tools::rbac::{CanIRequest, CanIService};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::ClusterTabs;

use super::dialog::{CanIDialog, CanIParts};
use super::memory::SessionMemory;

/// One `rbac::CanI` command, as the window receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanIOpen {
    /// The cluster to ask.
    pub cluster: ClusterId,
    /// The object to prefill from.
    pub target: Option<ResourceRef>,
    /// The verb to prefill.
    pub verb: Option<String>,
}

/// A handle on the window's can-i queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct CanIRequestSink {
    tx: UnboundedSender<CanIOpen>,
}

impl CanIRequestSink {
    /// A sink and the receiver [`CanIViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<CanIOpen>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to open the dialog. Fails when the window is gone.
    pub fn send(&self, open: CanIOpen) -> OxiResult<()> {
        self.tx
            .unbounded_send(open)
            .map_err(|_| OxiError::internal("the window with the cluster tabs is gone"))
    }
}

/// Registers `rbac::CanI` (and its MCP tool stub, `k8s.rbac_can_i`) on `registry`. The binary
/// installs it as `oxikube_rbac_ui`.
///
/// # Errors
///
/// A [`RegisterError`] when the id is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: CanIRequestSink,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::RBAC_CAN_I)
        .ok_or(RegisterError::Undeclared(CommandId::RBAC_CAN_I))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            let Command::RbacCanI {
                cluster,
                target,
                verb,
            } = command
            else {
                return Err(OxiError::validation("not a rbac::CanI command"));
            };
            sink.send(CanIOpen {
                cluster,
                target,
                verb,
            })?;
            Ok(CommandOutput::none())
        }
    })
}

/// What opening the dialog needs.
#[derive(Clone)]
pub struct CanIViewsDeps {
    /// Asks the clusters.
    pub service: CanIService,
    /// The window's cluster tabs.
    pub tabs: WeakEntity<ClusterTabs>,
}

/// The window's can-i dialogs.
pub struct CanIViews;

impl CanIViews {
    /// Starts applying the requests of `requests` on the UI thread. Keep the returned task for the
    /// life of the window.
    pub fn start(
        deps: CanIViewsDeps,
        mut requests: UnboundedReceiver<CanIOpen>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        let memory = SessionMemory::new();
        window.spawn(cx, async move |cx| {
            while let Some(open) = requests.next().await {
                let shown = cx.update(|window, cx| Self::open(&deps, &memory, open, window, cx));
                if shown.is_err() {
                    break;
                }
            }
        })
    }

    /// Shows the dialog in `open.cluster`'s tab. `false` when the cluster has no tab here.
    pub fn open(
        deps: &CanIViewsDeps,
        memory: &SessionMemory,
        open: CanIOpen,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(tabs) = deps.tabs.upgrade() else {
            return false;
        };
        let Some(workspace) = tabs
            .read(cx)
            .tab(&open.cluster)
            .map(|tab| tab.read(cx).workspace().clone())
        else {
            tracing::debug!(cluster = %open.cluster, "rbac::CanI: the cluster has no tab in this window");
            return false;
        };
        tabs.update(cx, |tabs, cx| {
            tabs.activate(&open.cluster, window, cx);
        });
        let prefill = match &open.target {
            Some(target) => deps
                .service
                .prefill(&open.cluster, target, open.verb.as_deref()),
            None => CanIRequest {
                verb: open.verb.clone().unwrap_or_default(),
                ..CanIRequest::default()
            },
        };
        let parts = CanIParts {
            cluster: open.cluster,
            service: deps.service.clone(),
            prefill,
            memory: memory.clone(),
        };
        workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| CanIDialog::new(parts, window, cx));
            workspace.show_modal(dialog, window, cx);
        });
        true
    }
}
