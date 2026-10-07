//! [`LogViews`]: one per window; opens log views in their cluster's tab and applies the
//! `logs::*` requests to them, on the UI thread.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, AppContext as _, Context, Entity, SharedString, Task, WeakEntity, Window};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_workspace::{ClusterTabs, OpenOptions, Workspace};

use super::{LogRequest, ViewChange};
use crate::LogsSettings;
use crate::search::SearchMemory;
use crate::view::{LogView, LogViewDeps, OpenLogs, ViewOptions, item_key};

/// Where a cluster's log views live: the workspace of its tab in this window. The app's is the
/// window's [`ClusterTabs`] (`WeakEntity<ClusterTabs>` implements it); a test hands a workspace.
pub trait LogHost: 'static {
    /// The workspace of `cluster`'s tab, `None` when it has no tab here.
    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>>;

    /// Shows `cluster`'s tab (a log view is opening in it).
    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App);
}

impl LogHost for WeakEntity<ClusterTabs> {
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

/// What [`LogViews`] needs.
#[derive(Clone)]
pub struct LogViewsDeps {
    /// What every log view is built over.
    pub views: LogViewDeps,
    /// Where the views of a cluster live (the window's cluster tabs).
    pub host: Rc<dyn LogHost>,
}

/// Opens and drives the log views of one window. See the [module docs](self).
pub struct LogViews {
    deps: LogViewsDeps,
    /// The searches of this window's log views, kept for the session.
    searches: SearchMemory,
    _requests: Task<()>,
}

impl LogViews {
    /// Starts the controller over `requests` (the receiver of the window's
    /// [`LogCommandSink`](super::LogCommandSink)).
    pub fn start(
        deps: LogViewsDeps,
        mut requests: UnboundedReceiver<LogRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let pump = cx.spawn_in(window, async move |this, cx| {
                while let Some(request) = requests.next().await {
                    let applied = this.update_in(cx, |views: &mut Self, window, cx| {
                        views.apply(request, window, cx)
                    });
                    if applied.is_err() {
                        break;
                    }
                }
            });
            Self {
                deps,
                searches: SearchMemory::new(),
                _requests: pump,
            }
        })
    }

    /// Applies one request.
    pub fn apply(&mut self, request: LogRequest, window: &mut Window, cx: &mut Context<Self>) {
        match request {
            LogRequest::Open { target, open } => {
                self.open(&target, &open, window, cx);
            }
            LogRequest::Change { target, change } => {
                if let Some(view) = self.view_of(&target, cx) {
                    view.update(cx, |view, cx| match &change {
                        ViewChange::SetRange(range) => view.set_range(*range, cx),
                        ViewChange::SelectContainer(name) => view.select_container(name, cx),
                        ViewChange::ToggleAutoscroll => view.toggle_autoscroll(cx),
                        ViewChange::ToggleFullscreen => view.toggle_fullscreen(window, cx),
                        ViewChange::TogglePrevious => view.toggle_previous(cx),
                        ViewChange::ToggleTimestamps => view.toggle_timestamps(cx),
                        ViewChange::ToggleWrap => view.toggle_wrap(cx),
                        ViewChange::Find(pattern) => view.find(pattern.as_deref(), window, cx),
                        ViewChange::NextMatch => view.next_match(cx),
                        ViewChange::PreviousMatch => view.previous_match(cx),
                        ViewChange::ToggleCase => view.toggle_case(cx),
                        ViewChange::ToggleInverse => view.toggle_inverse(cx),
                        ViewChange::ToggleFilterMode => view.toggle_filter_mode(cx),
                        ViewChange::CloseSearch => view.close_search(window, cx),
                        ViewChange::ToggleJsonMode => view.toggle_json_mode(cx),
                        ViewChange::ToggleLevel(chip) => view.toggle_level(*chip, cx),
                        ViewChange::ToggleLine(seq) => view.toggle_expanded(*seq, cx),
                        ViewChange::CollapseLine => view.collapse(cx),
                    });
                }
            }
        }
    }

    /// Shows the log view of `target` in its cluster's tab, reading what `open` asks: the open
    /// one (switched to it), else a new tab, focused so its keys work. `None` when the cluster
    /// has no tab here.
    pub fn open(
        &mut self,
        target: &ResourceRef,
        open: &OpenLogs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<LogView>> {
        let workspace = self.deps.host.workspace(&target.cluster, cx)?;
        self.deps.host.show(&target.cluster, window, cx);
        let key = SharedString::from(item_key(target));
        if let Some(id) = workspace.read(cx).find_item_by_key(&key, cx) {
            workspace.update(cx, |ws, cx| ws.activate_item(id, true, window, cx));
            let view = find(&workspace, target, cx)?;
            view.update(cx, |view, cx| view.open_logs(open, cx));
            return Some(view);
        }
        let deps = self.deps.views.clone();
        let target = target.clone();
        let mut options = ViewOptions::from_settings(&LogsSettings::resolve(&target.cluster, cx));
        open.apply(&mut options);
        let searches = self.searches.clone();
        let view = cx.new(|cx| {
            let saved = searches.get(&target);
            let mut view = LogView::with_options(target, options, deps, cx);
            view.set_workspace(workspace.downgrade());
            view.set_search_memory(searches);
            // The same pod's logs opened again in this session: the search as it was.
            if let Some(saved) = saved {
                view.restore_search(&saved, cx);
            }
            view
        });
        let options = OpenOptions {
            focus: true,
            ..OpenOptions::default()
        };
        workspace.update(cx, |ws, cx| {
            ws.open_item_with(Box::new(view.clone()), options, window, cx)
        });
        Some(view)
    }

    /// The open log view of `target` (in its cluster's tab).
    pub fn view_of(&self, target: &ResourceRef, cx: &App) -> Option<Entity<LogView>> {
        let workspace = self.deps.host.workspace(&target.cluster, cx)?;
        find(&workspace, target, cx)
    }
}

fn find(workspace: &Entity<Workspace>, target: &ResourceRef, cx: &App) -> Option<Entity<LogView>> {
    workspace
        .read(cx)
        .items_of_type::<LogView>()
        .into_iter()
        .find(|view| view.read(cx).target() == target)
}
