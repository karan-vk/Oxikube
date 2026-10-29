//! [`AuditViews`]: one per window; opens the audit viewer in a cluster tab and applies the
//! `audit::*` requests to it, on the UI thread.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, AppContext as _, Context, Entity, Task, WeakEntity, Window};
use oxikube_domain::ids::ClusterId;
use oxikube_workspace::{ClusterTabs, Toast, Workspace};

use super::commands::AuditRequest;
use super::view::{AuditView, AuditViewDeps};

/// Where the viewer lives: the cluster tabs of this window. The app's is the window's
/// [`ClusterTabs`] (`WeakEntity<ClusterTabs>` implements it); a test hands a workspace.
pub trait AuditHost: 'static {
    /// The tab the viewer opens in: `cluster`'s when it has one here, else the active tab, else
    /// any. `None` when no cluster tab is open.
    fn tab(&self, cluster: Option<&ClusterId>, cx: &App) -> Option<(ClusterId, Entity<Workspace>)>;

    /// Shows `cluster`'s tab (the viewer is opening in it).
    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App);

    /// The workspaces of every tab (to find an open viewer).
    fn workspaces(&self, cx: &App) -> Vec<Entity<Workspace>>;
}

impl AuditHost for WeakEntity<ClusterTabs> {
    fn tab(&self, cluster: Option<&ClusterId>, cx: &App) -> Option<(ClusterId, Entity<Workspace>)> {
        let tabs = self.upgrade()?;
        let tabs = tabs.read(cx);
        let id = cluster
            .filter(|id| tabs.tab(id).is_some())
            .cloned()
            .or_else(|| tabs.active().cloned())
            .or_else(|| tabs.clusters(cx).into_iter().next())?;
        let workspace = tabs.tab(&id)?.read(cx).workspace().clone();
        Some((id, workspace))
    }

    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App) {
        if let Some(tabs) = self.upgrade() {
            tabs.update(cx, |tabs, cx| {
                tabs.activate(cluster, window, cx);
            });
        }
    }

    fn workspaces(&self, cx: &App) -> Vec<Entity<Workspace>> {
        let Some(tabs) = self.upgrade() else {
            return Vec::new();
        };
        let tabs = tabs.read(cx);
        tabs.clusters(cx)
            .iter()
            .filter_map(|id| tabs.tab(id).map(|tab| tab.read(cx).workspace().clone()))
            .collect()
    }
}

/// What [`AuditViews`] needs.
#[derive(Clone)]
pub struct AuditViewsDeps {
    /// What the viewer is built over.
    pub view: AuditViewDeps,
    /// Where the viewer lives (the window's cluster tabs).
    pub host: Rc<dyn AuditHost>,
}

/// Opens and drives the audit viewer of one window.
pub struct AuditViews {
    deps: AuditViewsDeps,
    _requests: Task<()>,
}

impl AuditViews {
    /// Starts the controller over `requests` (the receiver of the window's
    /// [`AuditCommandSink`](super::AuditCommandSink)). Keep the entity for the life of the window.
    pub fn start(
        deps: AuditViewsDeps,
        mut requests: UnboundedReceiver<AuditRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx: &mut Context<Self>| {
            let pump = cx.spawn_in(window, async move |this, cx| {
                while let Some(request) = requests.next().await {
                    let applied = this.update_in(cx, |views: &mut Self, window, cx| {
                        views.apply(request, window, cx);
                    });
                    if applied.is_err() {
                        break;
                    }
                }
            });
            Self {
                deps,
                _requests: pump,
            }
        })
    }

    /// Applies `request`: `audit::Open` shows (or opens) the viewer; `audit::Export` exports the
    /// open viewer's filtered records.
    pub fn apply(&mut self, request: AuditRequest, window: &mut Window, cx: &mut Context<Self>) {
        match request {
            AuditRequest::Open { cluster } => self.open(cluster, window, cx),
            AuditRequest::Export { format } => {
                let Some(view) = self.open_view(cx) else {
                    self.toast_in_any_tab(
                        Toast::info("Open the audit log first (Audit Log in the palette)."),
                        cx,
                    );
                    return;
                };
                view.update(cx, |view, cx| view.export(format, cx));
            }
        }
    }

    fn open(&mut self, filter: Option<ClusterId>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((tab, workspace)) = self.deps.host.tab(filter.as_ref(), cx) else {
            tracing::debug!("audit::Open: no cluster tab is open to show the viewer in");
            return;
        };
        // The tab first: showing it focuses the tab itself, and the viewer must end up with the
        // focus so its keys work at once.
        self.deps.host.show(&tab, window, cx);
        let view = AuditView::open(&workspace, self.deps.view.clone(), window, cx);
        view.update(cx, |view, cx| {
            if filter.is_some() {
                view.set_cluster_filter(filter, cx);
            }
            view.focus_table(window, cx);
        });
    }

    /// The viewer an export acts on: the one in the active tab, else the first open in any tab.
    fn open_view(&self, cx: &App) -> Option<Entity<AuditView>> {
        let active = self
            .deps
            .host
            .tab(None, cx)
            .and_then(|(_, workspace)| AuditView::find(&workspace, cx));
        active.or_else(|| {
            self.deps
                .host
                .workspaces(cx)
                .iter()
                .find_map(|workspace| AuditView::find(workspace, cx))
        })
    }

    fn toast_in_any_tab(&self, toast: Toast, cx: &mut Context<Self>) {
        if let Some((_, workspace)) = self.deps.host.tab(None, cx) {
            workspace.update(cx, |workspace, cx| workspace.show_toast(toast, cx));
        }
    }
}
