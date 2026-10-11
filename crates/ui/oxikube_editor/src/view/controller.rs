//! [`EditorViews`]: one per window; applies the `editor::*` requests on the UI thread.
//!
//! - **New** opens an empty [`ManifestEditor`] as a tab of the shown cluster's workspace (its
//!   buffer checked against that cluster's schemas), or of the window's own workspace with syntax
//!   checks only when no cluster tab is shown. A request naming a cluster without a tab here does
//!   nothing.
//! - **Open** (`resource::Edit`, `resource::ViewYamlInEditor`, E10-S06) opens an editor on a live
//!   object in the object's cluster tab, titled `kind/name`, or focuses the tab that already
//!   holds the object. The object is read fresh, off the UI thread, by the editor itself.
//! - **ToggleReadOnly** / **ToggleSoftWrap** / **ToggleManagedFields** / **ToggleSecretMask** /
//!   **ConvertTo** / **UseLanguage** act on the focused editor, else the active pane's item when it
//!   is one. The toolbar's toggles
//!   focus their own editor before they send, so a click acts on the editor clicked.
//!
//! It also tells the user, with a warning toast, about an `editor` setting that was rejected
//! (E10-S12).
//!
//! Each request waits one turn first, so a palette that just closed has handed the focus back to
//! the editor it was opened over.

use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{
    App, AppContext as _, Context, Entity, Focusable as _, SharedString, Subscription, Task,
    WeakEntity, Window,
};
use oxikube_app::ClusterSessionManager;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_ports::ResourceReader;
use oxikube_settings::SettingsStore;
use oxikube_workspace::{ClusterTabs, CommandDispatcher, Workspace};

use super::commands::EditorRequest;
use super::manifest_editor::{ManifestEditor, ManifestEditorParts};
use super::schemas::{SchemaSource, SessionSchemas};
use crate::open::{OpenMode, ResourceOpen};
use crate::settings::notice;

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

    /// The reader to `get` objects of `cluster` with; `None` while it is not connected.
    fn reader(&self, _: &ClusterId) -> Option<Arc<dyn ResourceReader>> {
        None
    }

    /// Whether `cluster` is in read-only mode (an edit tab then opens read-only).
    fn cluster_read_only(&self, _: &ClusterId) -> bool {
        false
    }
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

    fn reader(&self, cluster: &ClusterId) -> Option<Arc<dyn ResourceReader>> {
        self.sessions.get(cluster)?.resources()
    }

    fn cluster_read_only(&self, cluster: &ClusterId) -> bool {
        self.sessions
            .get(cluster)
            .is_some_and(|session| session.read_only())
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
    /// The setting problems the last notice was about, so an unrelated change of the settings
    /// does not show it again.
    reported: Vec<String>,
    _requests: Task<()>,
    /// Watches the settings for values the editor rejected (E10-S12).
    _settings: Subscription,
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
            // A bad value in `settings.json` is told once when the window opens (after the
            // workspace exists) and again whenever the list of problems changes.
            cx.defer_in(window, |this: &mut Self, _, cx| {
                this.report_setting_problems(cx)
            });
            let settings = cx.observe_global::<SettingsStore>(|this, cx| {
                this.report_setting_problems(cx);
            });
            Self {
                deps,
                opened: 0,
                reported: Vec::new(),
                _requests: pump,
                _settings: settings,
            }
        })
    }

    /// Applies `request` on the next turn (see the module docs).
    pub fn apply(&mut self, request: EditorRequest, window: &mut Window, cx: &mut Context<Self>) {
        cx.defer_in(window, move |this, window, cx| match request {
            EditorRequest::New { cluster } => this.open_new(cluster, window, cx),
            EditorRequest::Open { target, mode } => this.open_resource(&target, mode, window, cx),
            EditorRequest::ToggleManagedFields => {
                this.on_target(window, cx, |editor, window, cx| {
                    editor.toggle_managed_fields(window, cx);
                })
            }
            EditorRequest::ToggleSecretMask => this.on_target(window, cx, |editor, window, cx| {
                editor.toggle_secret_mask(window, cx);
            }),
            EditorRequest::ToggleReadOnly => this.on_target(window, cx, |editor, window, cx| {
                editor.toggle_read_only(window, cx);
            }),
            EditorRequest::ToggleSoftWrap => this.on_target(window, cx, |editor, window, cx| {
                editor.toggle_soft_wrap(window, cx);
            }),
            EditorRequest::ConvertTo(language) => {
                this.on_target(window, cx, move |editor, window, cx| {
                    editor.convert_to(language, window, cx);
                });
            }
            EditorRequest::UseLanguage(language) => {
                this.on_target(window, cx, move |editor, window, cx| {
                    editor.use_language(language, window, cx);
                });
            }
        });
    }

    /// Shows a notice for the editor settings the store rejected, unless it already has.
    fn report_setting_problems(&mut self, cx: &mut Context<Self>) {
        let problems = notice::problems(cx);
        if problems == self.reported {
            return;
        }
        self.reported = problems;
        let Some(toast) = notice::notice(&self.reported) else {
            return;
        };
        if let Some(workspace) = self.deps.window_workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                workspace.show_toast(toast, cx);
            });
        }
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
            resource: None,
        };
        let editor = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        workspace.update(cx, |ws, cx| ws.open_item(editor, window, cx));
    }

    /// Opens an editor on `target` in its cluster's tab, or focuses the one that holds it (and
    /// makes it editable when this is "Edit" on a tab opened read-only). A target whose cluster
    /// has no tab in this window does nothing.
    fn open_resource(
        &mut self,
        target: &ResourceRef,
        mode: OpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.deps.host.clone();
        let cluster = &target.cluster;
        let Some(workspace) = host.workspace(cluster, cx) else {
            tracing::debug!(%target, "the editor cannot open: the cluster has no tab in this window");
            return;
        };
        host.show(cluster, window, cx);
        let held = workspace
            .read(cx)
            .items_of_type::<ManifestEditor>()
            .into_iter()
            .find(|editor| editor.read(cx).resource_ref() == Some(target));
        if let Some(editor) = held {
            let reader = host.reader(cluster);
            editor.update(cx, |editor, cx| editor.reopen(mode, reader, window, cx));
            workspace.update(cx, |ws, cx| {
                ws.activate_item(editor.entity_id(), true, window, cx);
            });
            return;
        }
        let parts = ManifestEditorParts {
            // The editor titles itself `kind/name` from the binding.
            title: SharedString::default(),
            text: String::new(),
            schemas: Some(host.schemas(cluster)),
            dispatcher: self.deps.dispatcher.clone(),
            resource: Some(ResourceOpen {
                target: target.clone(),
                mode,
                reader: host.reader(cluster),
                cluster_read_only: host.cluster_read_only(cluster),
            }),
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
