//! `cluster::ShowOverview`: open the overview in the cluster's tab.
//!
//! The bus handler ([`register_commands`]) queues the cluster on an [`OverviewRequestSink`]; the
//! window's [`OverviewViews`] drains the queue on the UI thread, opens (or shows again) the
//! overview as an item of that cluster's own workspace and brings the cluster's tab forward. The
//! handler only names the cluster: nothing is read from it.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::ClusterTabs;

use crate::overview::{ClusterOverview, OverviewDeps};

/// A handle on the window's overview-request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct OverviewRequestSink {
    tx: UnboundedSender<ClusterId>,
}

impl OverviewRequestSink {
    /// A sink and the receiver [`OverviewViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<ClusterId>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to show `cluster`'s overview. Fails when the window is gone.
    pub fn send(&self, cluster: ClusterId) -> OxiResult<()> {
        self.tx
            .unbounded_send(cluster)
            .map_err(|_| OxiError::internal("the window with the cluster tabs is gone"))
    }
}

/// Registers `cluster::ShowOverview` (and its MCP tool stub, `app.cluster_show_overview`) on
/// `registry`. The binary installs it as `oxikube_overview_ui`.
///
/// # Errors
///
/// A [`RegisterError`] when the id is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: OverviewRequestSink,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::CLUSTER_SHOW_OVERVIEW)
        .ok_or(RegisterError::Undeclared(CommandId::CLUSTER_SHOW_OVERVIEW))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            let Command::ClusterShowOverview { cluster } = command else {
                return Err(OxiError::validation("not a cluster::ShowOverview command"));
            };
            sink.send(cluster)?;
            Ok(CommandOutput::none())
        }
    })
}

/// The window's overviews.
pub struct OverviewViews;

impl OverviewViews {
    /// Starts applying the requests of `requests` on the UI thread, opening overviews built from
    /// `deps`. Keep the returned task for the life of the window (dropping it stops it).
    pub fn start(
        deps: OverviewDeps,
        tabs: WeakEntity<ClusterTabs>,
        mut requests: UnboundedReceiver<ClusterId>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            while let Some(cluster) = requests.next().await {
                let shown = cx.update(|window, cx| Self::open(&deps, &tabs, &cluster, window, cx));
                match shown {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::debug!(%cluster, "cluster::ShowOverview: the cluster has no tab in this window");
                    }
                    Err(_) => break,
                }
            }
        })
    }

    /// Shows the overview of `cluster` in its tab and brings the tab forward. `false` when the
    /// cluster has no tab in this window.
    pub fn open(
        deps: &OverviewDeps,
        tabs: &WeakEntity<ClusterTabs>,
        cluster: &ClusterId,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(tabs) = tabs.upgrade() else {
            return false;
        };
        let Some(workspace) = tabs
            .read(cx)
            .tab(cluster)
            .map(|tab| tab.read(cx).workspace().clone())
        else {
            return false;
        };
        // The tab is brought forward next, so the new overview starts shown.
        ClusterOverview::open(&workspace, cluster.clone(), deps.clone(), true, window, cx);
        tabs.update(cx, |tabs, cx| {
            tabs.activate(cluster, window, cx);
        });
        true
    }
}
