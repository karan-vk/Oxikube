//! `cluster::ShowMetricsSettings`: open the page in the cluster's tab.
//!
//! The bus handler ([`register_commands`]) queues the cluster on a [`MetricsRequestSink`]; the
//! window's [`MetricsViews`] drains the queue on the UI thread, opens (or shows again) the page
//! as an item of that cluster's own workspace and brings the cluster's tab forward. Nothing here
//! reads a cluster: the handler only names the cluster to show.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, AppContext as _, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::{ClusterTabs, CommandDispatcher, OpenOptions};

use super::backend::MetricsBackend;
use super::view::{ITEM_KEY, MetricsDeps, MetricsSettingsView};

/// A handle on the window's page-request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct MetricsRequestSink {
    tx: UnboundedSender<ClusterId>,
}

impl MetricsRequestSink {
    /// A sink and the receiver [`MetricsViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<ClusterId>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to show `cluster`'s page. Fails when the window is gone.
    pub fn send(&self, cluster: ClusterId) -> OxiResult<()> {
        self.tx
            .unbounded_send(cluster)
            .map_err(|_| OxiError::internal("the window with the cluster tabs is gone"))
    }
}

/// Registers `cluster::ShowMetricsSettings` (and its MCP tool stub,
/// `app.cluster_show_metrics_settings`) on `registry`. The binary installs it as
/// `oxikube_settings_ui`.
///
/// # Errors
///
/// A [`RegisterError`] when the id is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: MetricsRequestSink,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::CLUSTER_SHOW_METRICS_SETTINGS).ok_or(
        RegisterError::Undeclared(CommandId::CLUSTER_SHOW_METRICS_SETTINGS),
    )?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            let Command::ClusterShowMetricsSettings { cluster } = command else {
                return Err(OxiError::validation(
                    "not a cluster::ShowMetricsSettings command",
                ));
            };
            sink.send(cluster)?;
            Ok(CommandOutput::none())
        }
    })
}

/// What opening the page needs.
#[derive(Clone)]
pub struct MetricsViewsDeps {
    /// Where the page reads its status.
    pub backend: Rc<dyn MetricsBackend>,
    /// Where the page's buttons send their commands (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The window's cluster tabs.
    pub tabs: WeakEntity<ClusterTabs>,
}

/// The window's metrics pages.
pub struct MetricsViews;

impl MetricsViews {
    /// Starts applying the requests of `requests` on the UI thread. Keep the returned task for the
    /// life of the window (dropping it stops it).
    pub fn start(
        deps: MetricsViewsDeps,
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
                        tracing::debug!(%cluster, "cluster::ShowMetricsSettings: the cluster has no tab in this window");
                    }
                    Err(_) => break,
                }
            }
        })
    }

    /// Shows the page of `cluster` in its tab (the open one when there is one) and brings the tab
    /// forward. `false` when the cluster has no tab in this window.
    pub fn open(
        deps: &MetricsViewsDeps,
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
        let key = gpui::SharedString::from(ITEM_KEY);
        let existing = workspace.read(cx).find_item_by_key(&key, cx);
        match existing {
            Some(item) => workspace.update(cx, |ws, cx| ws.activate_item(item, true, window, cx)),
            None => {
                let page = MetricsDeps {
                    backend: deps.backend.clone(),
                    dispatcher: deps.dispatcher.clone(),
                };
                let view = cx.new(|cx| MetricsSettingsView::new(cluster.clone(), page, cx));
                let options = OpenOptions {
                    focus: true,
                    reuse_existing: true,
                    ..OpenOptions::default()
                };
                workspace.update(cx, |ws, cx| {
                    ws.open_item_with(Box::new(view), options, window, cx);
                });
            }
        }
        tabs.update(cx, |tabs, cx| {
            tabs.activate(cluster, window, cx);
        });
        true
    }
}
