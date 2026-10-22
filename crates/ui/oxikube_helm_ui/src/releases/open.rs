//! `helm::OpenReleases`: open the Helm page of a cluster in its tab.
//!
//! The bus handler ([`register_commands`]) queues the cluster on a [`HelmReleasesRequestSink`];
//! the window's [`HelmReleasesViews`] drains the queue on the UI thread, opens (or shows again)
//! the [`HelmPage`] as an item of that cluster's own workspace, with the releases view inside it,
//! and brings the cluster's tab forward. The sidebar's Helm > Releases entry and the `:helm` jump
//! send the same command.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, AppContext as _, SharedString, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::{ClusterTabs, CommandDispatcher, OpenOptions};

use super::backend::ReleasesBackend;
use super::view::{HelmReleasesView, ReleasesDeps};
use crate::gate::HelmGateState;
use crate::page::{HelmPage, ITEM_KEY};

/// A handle on the window's page-request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct HelmReleasesRequestSink {
    tx: UnboundedSender<ClusterId>,
}

impl HelmReleasesRequestSink {
    /// A sink and the receiver [`HelmReleasesViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<ClusterId>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to show `cluster`'s releases. Fails when the window is gone.
    pub fn send(&self, cluster: ClusterId) -> OxiResult<()> {
        self.tx
            .unbounded_send(cluster)
            .map_err(|_| OxiError::internal("the window with the cluster tabs is gone"))
    }
}

/// Registers `helm::OpenReleases` (and its tool stub, `helm.open_releases`) on `registry`. The
/// binary installs it as `oxikube_helm_ui::releases`.
///
/// # Errors
///
/// A [`RegisterError`] when the id is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: HelmReleasesRequestSink,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::HELM_OPEN_RELEASES)
        .ok_or(RegisterError::Undeclared(CommandId::HELM_OPEN_RELEASES))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            let Command::HelmOpenReleases { cluster } = command else {
                return Err(OxiError::validation("not a helm::OpenReleases command"));
            };
            sink.send(cluster)?;
            Ok(CommandOutput::none())
        }
    })
}

/// Makes the backend that serves one cluster's releases.
pub type ReleasesBackends = Rc<dyn Fn(&ClusterId) -> Rc<dyn ReleasesBackend>>;

/// What opening the page needs.
#[derive(Clone)]
pub struct HelmReleasesViewsDeps {
    /// The backend of each cluster.
    pub backends: ReleasesBackends,
    /// Where the page's buttons send their commands (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The window's cluster tabs.
    pub tabs: WeakEntity<ClusterTabs>,
}

/// The window's Helm releases pages.
pub struct HelmReleasesViews;

impl HelmReleasesViews {
    /// Starts applying the requests of `requests` on the UI thread. Keep the returned task for the
    /// life of the window (dropping it stops it).
    pub fn start(
        deps: HelmReleasesViewsDeps,
        mut requests: UnboundedReceiver<ClusterId>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            while let Some(cluster) = requests.next().await {
                match cx.update(|window, cx| Self::open(&deps, &cluster, window, cx)) {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::debug!(%cluster, "helm::OpenReleases: the cluster has no tab in this window");
                    }
                    Err(_) => break,
                }
            }
        })
    }

    /// Shows the Helm page of `cluster` in its tab (the open one when there is one) and brings the
    /// tab forward. `false` when the cluster has no tab in this window.
    pub fn open(
        deps: &HelmReleasesViewsDeps,
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
        let key = SharedString::from(ITEM_KEY);
        match workspace.read(cx).find_item_by_key(&key, cx) {
            Some(item) => workspace.update(cx, |ws, cx| ws.activate_item(item, true, window, cx)),
            None => {
                let gate = HelmGateState::install(cx);
                let releases = ReleasesDeps {
                    backend: (deps.backends)(cluster),
                    dispatcher: deps.dispatcher.clone(),
                    gate: gate.clone(),
                };
                let view = cx.new(|cx| HelmReleasesView::new(releases, window, cx));
                let dispatcher = deps.dispatcher.clone();
                let page = cx.new(|cx| HelmPage::with_releases(gate, dispatcher, view, cx));
                let options = OpenOptions {
                    focus: true,
                    reuse_existing: true,
                    ..OpenOptions::default()
                };
                workspace.update(cx, |ws, cx| {
                    ws.open_item_with(Box::new(page), options, window, cx);
                });
            }
        }
        true
    }
}
