//! [`EditorViews`]: one per window; applies the `editor::*` requests on the UI thread.
//!
//! - **New** opens an empty [`ManifestEditor`] as a tab of the shown cluster's workspace (its
//!   buffer checked against that cluster's schemas), or of the window's own workspace with syntax
//!   checks only when no cluster tab is shown. A request naming a cluster without a tab here does
//!   nothing.
//! - **ToggleReadOnly** / **ToggleSoftWrap** act on the focused editor, else the active pane's
//!   item when it is one.
//!
//! Each request waits one turn first, so a palette that just closed has handed the focus back to
//! the editor it was opened over.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, AppContext as _, Context, Entity, Focusable as _, Task, WeakEntity, Window};
use oxikube_app::ClusterSessionManager;
use oxikube_domain::ids::ClusterId;
use oxikube_workspace::{ClusterTabs, CommandDispatcher, Workspace};

use super::commands::EditorRequest;
use super::manifest_editor::{ManifestEditor, ManifestEditorParts};
use super::schemas::{SchemaSource, SessionSchemas};

/// Where a window's editors go, and the schemas of a cluster.
pub trait EditorHost: 'static {
    /// The cluster whose tab is displayed, if one is.
    fn active_cluster(&self, cx: &App) -> Option<ClusterId>;

    /// The workspace of `cluster`'s tab, `None` when it has no tab here.
    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>>;

    /// Shows `cluster`'s tab (an editor is opening in it).
    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App);

    /// The schemas of `cluster`.
    fn schemas(&self, cluster: &ClusterId) -> Rc<dyn SchemaSource>;
}

/// The app's [`EditorHost`]: the window's [`ClusterTabs`] and the sessions' schema ports.
#[derive(Clone)]
pub struct ClusterEditorHost {
    tabs: WeakEntity<ClusterTabs>,
    sessions: ClusterSessionManager,
}

impl ClusterEditorHost {
    /// A host over the window's cluster tabs.
    pub fn new(tabs: WeakEntity<ClusterTabs>, sessions: ClusterSessionManager) -> Self {
        Self { tabs, sessions }
    }
}

impl EditorHost for ClusterEditorHost {
    fn active_cluster(&self, cx: &App) -> Option<ClusterId> {
        self.tabs.upgrade()?.read(cx).active().cloned()
    }

    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>> {
        let tabs = self.tabs.upgrade()?;
        let tab = tabs.read(cx).tab(cluster)?;
        Some(tab.read(cx).workspace().clone())
    }

    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App) {
        if let Some(tabs) = self.tabs.upgrade() {
            tabs.update(cx, |tabs, cx| {
                tabs.activate(cluster, window, cx);
            });
        }
    }

    fn schemas(&self, cluster: &ClusterId) -> Rc<dyn SchemaSource> {
        Rc::new(SessionSchemas::new(self.sessions.clone(), cluster.clone()))
    }
}

/// What [`EditorViews`] needs.
#[derive(Clone)]
pub struct EditorViewsDeps {
    /// Where the clusters' editors go (the window's cluster tabs).
    pub host: Rc<dyn EditorHost>,
    /// The window's own workspace: editors without a cluster open here.
    pub window_workspace: WeakEntity<Workspace>,
    /// Where the editors send their commands: the bus.
    pub dispatcher: Rc<dyn CommandDispatcher>,
}

/// Opens the window's manifest editors and applies their commands. See the module docs.
pub struct EditorViews {
    deps: EditorViewsDeps,
    /// How many editors this window has opened (the "Untitled-N" titles).
    opened: usize,
    _requests: Task<()>,
}

impl EditorViews {
    /// Starts the controller over `requests` (the receiver of the window's
    /// [`EditorViewSink`](super::EditorViewSink)).
    pub fn start(
        deps: EditorViewsDeps,
        mut requests: UnboundedReceiver<EditorRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| {
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
                opened: 0,
                _requests: pump,
            }
        })
    }

    /// Applies `request` on the next turn (see the module docs).
    pub fn apply(&mut self, request: EditorRequest, window: &mut Window, cx: &mut Context<Self>) {
        cx.defer_in(window, move |this, window, cx| match request {
            EditorRequest::New { cluster } => this.open_new(cluster, window, cx),
            EditorRequest::ToggleReadOnly => this.on_target(window, cx, |editor, window, cx| {
                editor.toggle_read_only(window, cx);
            }),
            EditorRequest::ToggleSoftWrap => this.on_target(window, cx, |editor, window, cx| {
                editor.toggle_soft_wrap(window, cx);
            }),
        });
    }

    /// Opens an empty editor: see the [module docs](self).
    fn open_new(
        &mut self,
        cluster: Option<ClusterId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let explicit = cluster.is_some();
        let cluster = cluster.or_else(|| self.deps.host.active_cluster(cx));
        let workspace = match &cluster {
            Some(cluster) => self.deps.host.workspace(cluster, cx),
            None => self.deps.window_workspace.upgrade(),
        };
        let Some(workspace) = workspace else {
            if explicit {
                tracing::debug!("editor::NewManifest: the cluster has no tab in this window");
            }
            return;
        };
        if let Some(cluster) = &cluster {
            self.deps.host.show(cluster, window, cx);
        }
        self.opened += 1;
        let parts = ManifestEditorParts {
            title: format!("Untitled-{}", self.opened).into(),
            text: String::new(),
            schemas: cluster.map(|cluster| self.deps.host.schemas(&cluster)),
            dispatcher: self.deps.dispatcher.clone(),
        };
        let editor = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        workspace.update(cx, |ws, cx| ws.open_item(editor, window, cx));
    }

    /// Runs `f` on the editor a toggle acts on, if there is one.
    fn on_target(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut ManifestEditor, &mut Window, &mut Context<ManifestEditor>),
    ) {
        let Some(workspace) = self.shown_workspace(cx) else {
            return;
        };
        if let Some(editor) = target_editor(&workspace, window, cx) {
            editor.update(cx, |editor, cx| f(editor, window, cx));
        }
    }

    /// The workspace on screen: the shown cluster's, else the window's.
    fn shown_workspace(&self, cx: &App) -> Option<Entity<Workspace>> {
        if let Some(cluster) = self.deps.host.active_cluster(cx)
            && let Some(workspace) = self.deps.host.workspace(&cluster, cx)
        {
            return Some(workspace);
        }
        self.deps.window_workspace.upgrade()
    }
}

/// The editor a toggle acts on: the focused one, else the active pane's item when it is one.
fn target_editor(
    workspace: &Entity<Workspace>,
    window: &Window,
    cx: &App,
) -> Option<Entity<ManifestEditor>> {
    let workspace = workspace.read(cx);
    workspace
        .items_of_type::<ManifestEditor>()
        .into_iter()
        .find(|editor| {
            editor
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
        })
        .or_else(|| workspace.active_item(cx)?.downcast::<ManifestEditor>())
}
