//! `secret::Create` on the window's [`EditorViews`]: the dialog, then the editor tab.

use std::rc::Rc;

use gpui::{AppContext as _, Context, Window};
use oxikube_domain::Resource;
use oxikube_domain::ids::ClusterId;
use serde_json::Value;

use super::dialog::{CreateSecretDialog, SecretSink};
use crate::open::{RenderOptions, render_resource};
use crate::view::controller::EditorViews;
use crate::view::manifest_editor::{ManifestEditor, ManifestEditorParts};
use crate::view::notice::{Notice, NoticeLevel};

/// What the new tab says about its text.
const TAB_NOTE: &str =
    "Values are shown as text and base64-encoded when applied. Review, then Apply.";

impl EditorViews {
    /// `secret::Create`: the "Create secret" dialog in `cluster`'s tab (the shown one when
    /// `None`), starting in the cluster's selected namespace. Nothing happens for a cluster
    /// without a tab here.
    pub(in crate::view) fn create_secret(
        &mut self,
        cluster: Option<ClusterId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.deps.host.clone();
        let Some(cluster) = cluster.or_else(|| host.active_cluster(cx)) else {
            self.warn("Open a cluster tab to create a secret", cx);
            return;
        };
        let Some(workspace) = host.workspace(&cluster, cx) else {
            tracing::debug!("secret::Create: the cluster has no tab in this window");
            return;
        };
        host.show(&cluster, window, cx);
        let selection = host.namespace_selection(&cluster);
        let namespace = selection.names().next().unwrap_or("default").to_owned();
        let views = cx.entity().downgrade();
        let choose: SecretSink = Rc::new(move |manifest, window, cx| {
            let cluster = cluster.clone();
            views
                .update(cx, |views, cx| {
                    views.open_secret_manifest(cluster, manifest, window, cx);
                })
                .ok();
        });
        workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| CreateSecretDialog::new(&namespace, choose, window, cx));
            workspace.show_modal(dialog, window, cx);
        });
    }

    /// Opens the new Secret `manifest` (`data` base64) in an editor on the next turn, as text:
    /// the dialog that made it is closing and hands the focus back to where it was.
    pub(in crate::view) fn open_secret_manifest(
        &mut self,
        cluster: ClusterId,
        manifest: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.defer_in(window, move |this, window, cx| {
            let host = this.deps.host.clone();
            let Some(workspace) = host.workspace(&cluster, cx) else {
                return;
            };
            let name = manifest
                .pointer("/metadata/name")
                .and_then(Value::as_str)
                .unwrap_or("secret")
                .to_owned();
            let Ok(resource) = Resource::from_json(manifest) else {
                tracing::warn!("the new Secret does not read as an object");
                return;
            };
            let options = RenderOptions {
                reveal_secrets: true,
                decode_secrets: true,
                ..RenderOptions::default()
            };
            let Ok(rendered) = render_resource(&resource, options) else {
                return;
            };
            let parts = ManifestEditorParts {
                title: format!("New Secret {name}").into(),
                text: rendered.text,
                schemas: Some(host.schemas(&cluster)),
                // A new object: there is nothing live to compare with.
                live: None,
                dispatcher: this.deps.dispatcher.clone(),
                resource: None,
            };
            let editor = cx.new(|cx| ManifestEditor::new(parts, window, cx));
            this.mount(&editor, Some(&cluster), &workspace, cx);
            editor.update(cx, |editor, cx| {
                editor.set_plaintext_secrets(true);
                editor.set_notice(Notice::new(NoticeLevel::Info, TAB_NOTE), cx);
            });
            workspace.update(cx, |ws, cx| ws.open_item(editor, window, cx));
        });
    }
}
