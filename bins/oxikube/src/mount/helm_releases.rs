//! The Helm releases page of the main window (E16-S05): `helm::OpenReleases` opens it in the
//! cluster's tab, and the sidebar's Helm > Releases entry and the `:helm` jump send the same
//! command.
//!
//! * [`SessionReleaseSource`] is what the app's [`ReleasesService`] reads: for each connected
//!   session a [`NativeHelmReader`] over the session's read-only resource port (so releases work
//!   with no `helm` binary), the session's namespace selection, its store and its discovery.
//! * [`start_views`] opens the page for each `helm::OpenReleases` request on the UI thread
//!   (`oxikube_helm_ui::releases::HelmReleasesViews`).
//! * [`install_tab`] runs for every cluster tab: a click on the sidebar's Releases entry (the
//!   `helm-releases` page) is the same command, with the tab's cluster.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use gpui::{App, Entity, Task, WeakEntity, Window};
use oxikube_app::helm::KindScopes;
use oxikube_app::{
    ClusterSession, ClusterSessionManager, ReleaseSource, ReleasesService, ResourceStore,
    ResourceStores, SessionUpdate,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_helm::NativeHelmReader;
use oxikube_helm_ui::releases::{
    HelmReleasesViews, HelmReleasesViewsDeps, ReleasesBackends, ServiceBackend,
};
use oxikube_ports::HelmPort;
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};
use oxikube_workspace::{ClusterTab, ClusterTabs, CommandDispatcher};
use parking_lot::Mutex;

use super::helm::HELM_PAGE;
use super::tabs::TabDeps;

/// The binary's [`ReleaseSource`]: sessions, stores and one native reader per connection.
pub struct SessionReleaseSource {
    sessions: ClusterSessionManager,
    stores: Arc<ResourceStores>,
    /// The reader of each cluster and the address of the resource port it reads through; a
    /// reconnect brings a new port, and so a new reader (the old reader's cache dies with it).
    readers: Mutex<HashMap<ClusterId, (usize, Arc<dyn HelmPort>)>>,
}

impl SessionReleaseSource {
    /// A source over `sessions` and their `stores`.
    pub fn new(sessions: ClusterSessionManager, stores: Arc<ResourceStores>) -> Self {
        Self {
            sessions,
            stores,
            readers: Mutex::new(HashMap::new()),
        }
    }

    fn connected(&self, cluster: &ClusterId) -> Option<ClusterSession> {
        self.sessions
            .get(cluster)
            .filter(ClusterSession::is_connected)
    }
}

impl ReleaseSource for SessionReleaseSource {
    fn helm(&self, cluster: &ClusterId) -> Option<Arc<dyn HelmPort>> {
        let session = self.connected(cluster)?;
        let resources = session.resources()?;
        let address = Arc::as_ptr(&resources).cast::<()>() as usize;
        let mut readers = self.readers.lock();
        if let Some((at, reader)) = readers.get(cluster)
            && *at == address
        {
            return Some(reader.clone());
        }
        let reader: Arc<dyn HelmPort> = Arc::new(NativeHelmReader::new(cluster.clone(), resources));
        readers.insert(cluster.clone(), (address, reader.clone()));
        Some(reader)
    }

    fn namespaces(&self, cluster: &ClusterId) -> NamespaceSelection {
        self.sessions
            .get(cluster)
            .map(|session| session.namespace_selection().clone())
            .unwrap_or_default()
    }

    fn store(&self, cluster: &ClusterId) -> Option<ResourceStore> {
        self.stores.for_session(&self.connected(cluster)?)
    }

    fn kinds(&self, cluster: &ClusterId) -> BoxFuture<'static, OxiResult<Arc<KindScopes>>> {
        let discovery = self.connected(cluster).and_then(|s| s.discovery());
        Box::pin(async move {
            let discovery =
                discovery.ok_or_else(|| OxiError::network("the cluster is not connected"))?;
            let kinds = discovery.discover().await?;
            Ok(Arc::new(KindScopes::from_kinds(&kinds)))
        })
    }

    fn session_updates(&self) -> BoxStream<'static, SessionUpdate> {
        self.sessions
            .subscribe()
            .filter_map(|update| async move { update.ok() })
            .boxed()
    }
}

/// The app's releases service over the open sessions.
pub fn service(sessions: ClusterSessionManager, stores: Arc<ResourceStores>) -> ReleasesService {
    ReleasesService::new(Arc::new(SessionReleaseSource::new(sessions, stores)))
}

/// Starts the window's releases pages over `requests` (the bus's `helm::OpenReleases` handler).
/// Keep the task for the life of the window.
pub fn start_views(
    service: ReleasesService,
    dispatcher: Rc<dyn CommandDispatcher>,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<ClusterId>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let backends: ReleasesBackends = Rc::new(move |cluster: &ClusterId| {
        Rc::new(ServiceBackend::new(service.clone(), cluster.clone()))
    });
    let deps = HelmReleasesViewsDeps {
        backends,
        dispatcher,
        tabs,
    };
    HelmReleasesViews::start(deps, requests, window, cx)
}

/// Routes a click on the sidebar's Releases entry to `helm::OpenReleases` for this tab's cluster.
pub fn install_tab(
    tab: &Entity<ClusterTab>,
    session: &ClusterSession,
    deps: &TabDeps,
    window: &mut Window,
    cx: &mut App,
) {
    let cluster = session.id().clone();
    let workspace = tab.read(cx).workspace().clone();
    let Some(panel) = workspace.read(cx).panel::<SidebarPanel>() else {
        tracing::warn!(%cluster, "the cluster tab has no sidebar: its Helm releases entry does not navigate");
        return;
    };
    let dispatcher = deps.dispatcher.clone();
    window
        .subscribe(&panel, cx, move |_, event: &SidebarEvent, _, cx| {
            let SidebarEvent::Navigate(SidebarTarget::Page(page)) = event else {
                return;
            };
            if page.as_ref() == HELM_PAGE {
                let cluster = cluster.clone();
                dispatcher.dispatch(Command::HelmOpenReleases { cluster }, cx);
            }
        })
        .detach();
}
