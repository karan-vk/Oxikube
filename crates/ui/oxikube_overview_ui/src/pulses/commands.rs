//! `cluster::ShowPulses`: open the page in the cluster's tab.
//!
//! The bus handler ([`register_commands`]) queues the cluster on a [`PulsesRequestSink`]; the
//! window's [`PulsesViews`] drains the queue on the UI thread, opens (or shows again) the page as
//! an item of that cluster's own workspace and brings the cluster's tab forward, the page focused. Nothing here
//! reads a cluster: the handler only names the cluster to show.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::ClusterTabs;

use super::view::{PulsesDeps, PulsesView};

/// A handle on the window's page-request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct PulsesRequestSink {
    tx: UnboundedSender<ClusterId>,
}

impl PulsesRequestSink {
    /// A sink and the receiver [`PulsesViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<ClusterId>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to show `cluster`'s pulses. Fails when the window is gone.
    pub fn send(&self, cluster: ClusterId) -> OxiResult<()> {
        self.tx
            .unbounded_send(cluster)
            .map_err(|_| OxiError::internal("the window with the cluster tabs is gone"))
    }
}

/// Registers `cluster::ShowPulses` (and its MCP tool stub, `app.cluster_show_pulses`) on
/// `registry`. The binary installs it as `oxikube_overview_ui`.
///
/// # Errors
///
/// A [`RegisterError`] when the id is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: PulsesRequestSink,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::CLUSTER_SHOW_PULSES)
        .ok_or(RegisterError::Undeclared(CommandId::CLUSTER_SHOW_PULSES))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            let Command::ClusterShowPulses { cluster } = command else {
                return Err(OxiError::validation("not a cluster::ShowPulses command"));
            };
            sink.send(cluster)?;
            Ok(CommandOutput::none())
        }
    })
}

/// What opening the page needs.
#[derive(Clone)]
pub struct PulsesViewsDeps {
    /// What the page is built from.
    pub page: PulsesDeps,
    /// The window's cluster tabs.
    pub tabs: WeakEntity<ClusterTabs>,
}

/// The window's pulses pages.
pub struct PulsesViews;

impl PulsesViews {
    /// Starts applying the requests of `requests` on the UI thread. Keep the returned task for
    /// the life of the window (dropping it stops it).
    pub fn start(
        deps: PulsesViewsDeps,
        mut requests: UnboundedReceiver<ClusterId>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            while let Some(cluster) = requests.next().await {
                let shown = cx.update(|window, cx| Self::open(&deps, &cluster, window, cx));
                match shown {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::debug!(%cluster, "cluster::ShowPulses: the cluster has no tab in this window");
                    }
                    Err(_) => break,
                }
            }
        })
    }

    /// Shows the page of `cluster` in its tab (the open one when there is one) and brings the
    /// tab forward. `false` when the cluster has no tab in this window.
    pub fn open(
        deps: &PulsesViewsDeps,
        cluster: &ClusterId,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(tabs) = deps.tabs.upgrade() else {
            return false;
        };
        let Some(workspace) = tabs
            .read(cx)
            .tab(cluster)
            .map(|tab| tab.read(cx).workspace().clone())
        else {
            return false;
        };
        // The tab first: showing it focuses the tab itself, and the page must end up with the
        // focus so its keys work at once.
        tabs.update(cx, |tabs, cx| {
            tabs.activate(cluster, window, cx);
        });
        PulsesView::open(&workspace, cluster.clone(), deps.page.clone(), window, cx);
        true
    }
}
