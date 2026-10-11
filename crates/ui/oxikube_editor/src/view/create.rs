//! Create from template (E10-S09) on the window's [`EditorViews`]: `resource::Create` lists the
//! templates, opens the "Create resource" picker, and opens the chosen template in a new editor.
//!
//! The listing reads the user's templates directory through `FsPort`, so it runs on the
//! Kubernetes runtime (`oxikube_runtime::spawn_kube`, abort-on-drop) and the picker opens when it
//! is done; the embedded templates are parsed on the background executor. The directory is read
//! again each time the picker opens, so a file added meanwhile is there. The new tab holds the
//! template with the cluster's selected namespace in its placeholders (`All` namespaces says so
//! in the notice bar) and no live object: the diff compares with nothing and the apply creates.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{AppContext as _, Context, Entity, Window, actions};
use oxikube_app::{Template, TemplateCatalog};
use oxikube_domain::ids::ClusterId;
use oxikube_workspace::{Toast, Workspace};

use super::controller::EditorViews;
use super::manifest_editor::{ManifestEditor, ManifestEditorParts};
use super::notice::{Notice, NoticeLevel};
use crate::templates::{TemplatePickerDelegate, TemplateSink, embedded};

actions!(
    resource,
    [
        /// Open the "Create resource" picker: a manifest for a core kind (or one of your own
        /// templates) with the selected namespace filled in (`resource::Create` on the bus).
        Create,
    ]
);

impl EditorViews {
    /// `resource::Create`: lists the templates, then opens the picker in `cluster`'s tab (the
    /// shown one when `None`), or, with a `template` name, the editor on that template. Nothing
    /// happens for a cluster without a tab here.
    pub(super) fn create_resource(
        &mut self,
        cluster: Option<ClusterId>,
        template: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.deps.host.clone();
        let Some(cluster) = cluster.or_else(|| host.active_cluster(cx)) else {
            self.warn("Open a cluster tab to create a resource", cx);
            return;
        };
        let Some(workspace) = host.workspace(&cluster, cx) else {
            tracing::debug!("resource::Create: the cluster has no tab in this window");
            return;
        };
        host.show(&cluster, window, cx);
        let service = host.templates();
        let user_dir = service
            .as_ref()
            .map(|service| service.user_dir().to_path_buf());
        // A newer request replaces this one, which cancels it.
        self.scan = Some(cx.spawn_in(window, async move |this, cx| {
            let catalog = match service {
                Some(service) => {
                    oxikube_runtime::spawn_kube(cx, async move { service.catalog().await })
                        .await
                        .ok()
                }
                None => None,
            };
            let catalog = match catalog {
                Some(catalog) => catalog,
                None => {
                    cx.background_spawn(async { TemplateCatalog::of(embedded()) })
                        .await
                }
            };
            let _ = this.update_in(cx, |views, window, cx| {
                views.templates_listed(cluster, workspace, template, catalog, user_dir, window, cx);
            });
        }));
    }

    /// The templates are listed: the editor on the one named, else the picker.
    #[allow(clippy::too_many_arguments, reason = "the listing's own results")]
    fn templates_listed(
        &mut self,
        cluster: ClusterId,
        workspace: Entity<Workspace>,
        template: Option<String>,
        catalog: TemplateCatalog,
        user_dir: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(name) = template {
            match catalog.get(&name).cloned() {
                Some(template) => {
                    self.open_template(cluster, template, window, cx);
                    return;
                }
                None => self.warn(format!("There is no template called {name}"), cx),
            }
        }
        let views = cx.entity().downgrade();
        let choose: TemplateSink = Rc::new(move |template, window, cx| {
            let cluster = cluster.clone();
            views
                .update(cx, |views, cx| {
                    views.open_template(cluster, template, window, cx);
                })
                .ok();
        });
        workspace.update(cx, |workspace, cx| {
            let picker =
                cx.new(|cx| TemplatePickerDelegate::picker(catalog, user_dir, choose, window, cx));
            workspace.show_modal(picker.clone(), window, cx);
            picker.update(cx, |picker, cx| picker.focus(window, cx));
        });
    }

    /// Opens an editor on `template` in `cluster`'s tab, on the next turn: the picker that chose
    /// it is closing and hands the focus back to where it was opened from, which the new tab
    /// must keep.
    pub(super) fn open_template(
        &mut self,
        cluster: ClusterId,
        template: Template,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.defer_in(window, move |this, window, cx| {
            let host = this.deps.host.clone();
            let Some(workspace) = host.workspace(&cluster, cx) else {
                return;
            };
            host.show(&cluster, window, cx);
            let prefill = template.prefill(&host.namespace_selection(&cluster));
            let parts = ManifestEditorParts {
                title: format!("New {}", template.name).into(),
                text: prefill.text,
                schemas: Some(host.schemas(&cluster)),
                live: host.live(&cluster),
                dispatcher: this.deps.dispatcher.clone(),
                resource: None,
            };
            let editor = cx.new(|cx| ManifestEditor::new(parts, window, cx));
            this.mount(&editor, Some(&cluster), &workspace, cx);
            if let Some(note) = prefill.note {
                editor.update(cx, |editor, cx| {
                    editor.set_notice(Notice::new(NoticeLevel::Info, note), cx);
                });
            }
            workspace.update(cx, |ws, cx| ws.open_item(editor, window, cx));
        });
    }

    /// A warning toast in the window's workspace.
    pub(in crate::view) fn warn(
        &self,
        text: impl Into<gpui::SharedString>,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.deps.window_workspace.upgrade() {
            let toast = Toast::warning(text);
            workspace.update(cx, |workspace, cx| {
                workspace.show_toast(toast, cx);
            });
        }
    }
}
