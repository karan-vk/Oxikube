//! [`EventsViews`]: one per window; opens the events views in their cluster's tab and applies the
//! `events::*` requests to them, on the UI thread.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, AppContext as _, Context, Entity, Task, WeakEntity, Window};
use oxikube_domain::ids::ClusterId;
use oxikube_workspace::{ClusterTabs, Workspace};

use super::{EventsRequest, ViewChange};
use crate::view::{EventsView, EventsViewDeps};

/// Where a cluster's events view lives: the workspace of its tab in this window. The app's is the
/// window's [`ClusterTabs`] (`WeakEntity<ClusterTabs>` implements it); a test hands a workspace.
pub trait EventsHost: 'static {
    /// The workspace of `cluster`'s tab, `None` when it has no tab here.
    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>>;

    /// Shows `cluster`'s tab (its events view is opening).
    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App);
}

impl EventsHost for WeakEntity<ClusterTabs> {
    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>> {
        let tabs = self.upgrade()?;
        let tab = tabs.read(cx).tab(cluster)?;
        Some(tab.read(cx).workspace().clone())
    }

    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App) {
        if let Some(tabs) = self.upgrade() {
            tabs.update(cx, |tabs, cx| {
                tabs.activate(cluster, window, cx);
            });
        }
    }
}

/// What [`EventsViews`] needs.
#[derive(Clone)]
pub struct EventsViewsDeps {
    /// What every events view is built over.
    pub view: EventsViewDeps,
    /// Where the views of a cluster live (the window's cluster tabs).
    pub host: Rc<dyn EventsHost>,
}

/// Opens and drives the events views of one window.
pub struct EventsViews {
    deps: EventsViewsDeps,
    _requests: Task<()>,
}

impl EventsViews {
    /// Starts the controller over `requests` (the receiver of the window's
    /// [`EventsCommandSink`](super::EventsCommandSink)). Keep the entity for the life of the
    /// window.
    pub fn start(
        deps: EventsViewsDeps,
        mut requests: UnboundedReceiver<EventsRequest>,
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

    /// Applies `request`: `events::Open` shows (or opens) the cluster's view; the rest act on the
    /// open view, and do nothing when the cluster has none here.
    pub fn apply(&mut self, request: EventsRequest, window: &mut Window, cx: &mut Context<Self>) {
        let EventsRequest { cluster, change } = request;
        let Some(workspace) = self.deps.host.workspace(&cluster, cx) else {
            tracing::debug!(%cluster, "events: the cluster has no tab in this window");
            return;
        };
        if let ViewChange::Open { namespace, filter } = change {
            // The tab first: showing it focuses the tab itself, and the view must end up with
            // the focus so its keys work at once.
            self.deps.host.show(&cluster, window, cx);
            let view = EventsView::open(&workspace, cluster, self.deps.view.clone(), window, cx);
            view.update(cx, |view, cx| {
                view.apply_open(namespace, filter, window, cx);
                view.focus_table(window, cx);
            });
            return;
        }
        let Some(view) = EventsView::find(&workspace, cx) else {
            tracing::debug!(%cluster, ?change, "events: no events view open for the cluster");
            return;
        };
        view.update(cx, |view, cx| match change {
            ViewChange::ToggleFaults => view.toggle_faults(cx),
            ViewChange::TogglePause => view.toggle_pause(cx),
            ViewChange::CopyRow => view.copy_selected(false, cx),
            ViewChange::CopyRowJson => view.copy_selected(true, cx),
            ViewChange::ExportJson => view.export(cx),
            ViewChange::Open { .. } => {}
        });
    }
}
