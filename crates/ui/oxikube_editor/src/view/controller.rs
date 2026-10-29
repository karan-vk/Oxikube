//! [`EditorViews`]: one per window; applies the `editor::*` requests on the UI thread.
//!
//! - **New** opens an empty [`ManifestEditor`] as a tab of the shown cluster's workspace (its
//!   buffer checked against that cluster's schemas), or of the window's own workspace with syntax
//!   checks only when no cluster tab is shown. A request naming a cluster without a tab here does
//!   nothing.
//! - **Create** (`resource::Create`, E10-S09) lists the templates and opens the "Create resource"
//!   picker (`create`); choosing one opens an editor on it in the cluster's tab.
//! - **Open** (`resource::Edit`, `resource::ViewYamlInEditor`, E10-S06) opens an editor on a live
//!   object in the object's cluster tab, titled `kind/name`, or focuses the tab that already
//!   holds the object. The object is read fresh, off the UI thread, by the editor itself.
//!   **OpenDecoded** (`secret::EditDecoded`, E12-S11) is Open in edit mode with a Secret's values
//!   as plaintext; **CreateSecret** (`secret::Create`) opens the "Create secret" dialog (`secret`).
//! - **ToggleReadOnly** / **ToggleSoftWrap** / **ToggleManagedFields** / **ToggleSecretMask** /
//!   **ConvertTo** / **UseLanguage**, **TriggerCompletion** / **ShowHover** (E10-S05), and the diff vs live requests (E10-S07: **ShowDiff**,
//!   **ReloadFromServer**, **MergeFromServer**, **KeepMine**, a side per conflict,
//!   **AcceptMerge**) act on the focused editor, else the active pane's item when it is one. The
//!   toolbar's toggles focus their own editor before they send, so a click acts on the editor
//!   clicked.
//!
//! It also tells the user, with a warning toast, about an `editor` setting that was rejected
//! (E10-S12).
//!
//! Each request waits one turn first, so a palette that just closed has handed the focus back to
//! the editor it was opened over.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{
    App, AppContext as _, Context, Entity, Focusable as _, SharedString, Subscription, Task,
    WeakEntity, Window,
};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_settings::SettingsStore;
use oxikube_workspace::{CommandDispatcher, Workspace};

use super::commands::EditorRequest;
use super::host::EditorHost;
use super::manifest_editor::{ManifestEditor, ManifestEditorParts};
use crate::open::{OpenMode, ResourceOpen};
use crate::settings::notice;

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
    pub(super) deps: EditorViewsDeps,
    /// How many editors this window has opened (the "Untitled-N" titles).
    pub(super) opened: usize,
    /// The templates scan behind the running `resource::Create`; a newer one replaces (and so
    /// cancels) it. Never cleared from inside itself.
    pub(super) scan: Option<Task<()>>,
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
                scan: None,
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
            EditorRequest::Create { cluster, template } => {
                this.create_resource(cluster, template, window, cx)
            }
            EditorRequest::Open { target, mode } => this.open_resource(&target, mode, window, cx),
            EditorRequest::OpenDecoded { target } => {
                this.open_resource(&target, OpenMode::EditDecoded, window, cx)
            }
            EditorRequest::CreateSecret { cluster } => this.create_secret(cluster, window, cx),
            EditorRequest::PickDiffTarget { target } => this.pick_diff_target(target, window, cx),
            EditorRequest::ShowResourceDiff { request, state } => {
                this.show_resource_diff(request, state, window, cx)
            }
            EditorRequest::OpenText(open) => this.open_text(open, window, cx),
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
            EditorRequest::TriggerCompletion => this.on_target(window, cx, |editor, window, cx| {
                editor.show_completions(window, cx);
            }),
            EditorRequest::ShowHover => this.on_target(window, cx, |editor, window, cx| {
                editor.show_hover(window, cx);
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
            request => this.on_target(window, cx, |editor, window, cx| {
                apply_live_request(editor, request, window, cx);
            }),
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
            schemas: cluster
                .as_ref()
                .map(|cluster| self.deps.host.schemas(cluster)),
            live: cluster
                .as_ref()
                .and_then(|cluster| self.deps.host.live(cluster)),
            dispatcher: self.deps.dispatcher.clone(),
            resource: None,
        };
        let editor = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        self.mount(&editor, cluster.as_ref(), &workspace, cx);
        workspace.update(cx, |ws, cx| ws.open_item(editor, window, cx));
    }

    /// What a new editor gets besides its parts: the apply pipeline of its cluster with the
    /// workspace its dialogs open in (E10-S08), and its place among the palette's command
    /// surfaces.
    pub(super) fn mount(
        &self,
        editor: &Entity<ManifestEditor>,
        cluster: Option<&ClusterId>,
        workspace: &Entity<Workspace>,
        cx: &mut App,
    ) {
        if let Some(service) = cluster.and_then(|cluster| self.deps.host.apply(cluster)) {
            let workspace = workspace.downgrade();
            editor.update(cx, |editor, _| editor.set_apply(service, workspace));
        }
        let focus = editor.read(cx).editor().focus_handle(cx);
        oxikube_workspace::command_surface::register(editor, &focus, cx);
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
            live: host.live(cluster),
            dispatcher: self.deps.dispatcher.clone(),
            resource: Some(ResourceOpen {
                target: target.clone(),
                mode,
                reader: host.reader(cluster),
                cluster_read_only: host.cluster_read_only(cluster),
            }),
        };
        let editor = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        self.mount(&editor, Some(cluster), &workspace, cx);
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

/// The diff vs live requests (E10-S07) on `editor`.
fn apply_live_request(
    editor: &mut ManifestEditor,
    request: EditorRequest,
    window: &mut Window,
    cx: &mut Context<ManifestEditor>,
) {
    match request {
        EditorRequest::ShowDiff => editor.toggle_diff(window, cx),
        EditorRequest::ToggleDiffLayout => editor.toggle_diff_layout(cx),
        EditorRequest::ReloadFromServer => editor.reload_from_server(window, cx),
        EditorRequest::MergeFromServer => editor.merge_from_server(window, cx),
        EditorRequest::KeepMine => editor.keep_mine(cx),
        EditorRequest::Take { conflict, side } => editor.take_side(conflict, side, cx),
        EditorRequest::AcceptMerge => editor.accept_merge(window, cx),
        _ => {}
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
