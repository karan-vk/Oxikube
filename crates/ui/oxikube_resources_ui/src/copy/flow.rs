//! [`CopyFlow`]: from "Copy to…" on a row to the cluster picker, the namespace picker and the
//! review dialog, one modal after the other in the cluster tab's workspace.

use gpui::{App, AppContext as _, WeakEntity, Window};
use oxikube_app::{ClusterChoice, CopyService};
use oxikube_domain::ids::ResourceRef;
use oxikube_workspace::{Toast, Workspace};

use super::cluster_picker::ClusterPickerDelegate;
use super::dialog::CopyDialog;
use super::namespace_picker::NamespacePickerDelegate;

/// What the steps before the review have decided.
#[derive(Clone, Debug)]
pub(super) struct CopyDraft {
    /// The object to copy.
    pub(super) source: ResourceRef,
    /// The target cluster, once chosen.
    pub(super) cluster: Option<ClusterChoice>,
}

/// Opens the copy flow for an object in a cluster tab's workspace. Cheap to clone. See the
/// [module docs](super).
#[derive(Clone)]
pub struct CopyFlow {
    pub(super) service: CopyService,
    pub(super) workspace: WeakEntity<Workspace>,
}

impl CopyFlow {
    /// A flow over `service` that asks in `workspace`.
    pub fn new(service: CopyService, workspace: WeakEntity<Workspace>) -> Self {
        Self { service, workspace }
    }

    /// Opens the cluster picker for `source` at once; it lists the clusters in the background.
    /// Does nothing without a live workspace. Returns whether a picker opened.
    pub fn begin(&self, source: ResourceRef, window: &mut Window, cx: &mut App) -> bool {
        let Some(workspace) = self.workspace.upgrade() else {
            return false;
        };
        let draft = CopyDraft {
            source,
            cluster: None,
        };
        let flow = self.clone();
        workspace.update(cx, |workspace, cx| {
            let picker = cx.new(|cx| ClusterPickerDelegate::picker(flow, draft, window, cx));
            workspace.show_modal(picker.clone(), window, cx);
            picker.update(cx, |picker, cx| picker.focus(window, cx));
        });
        true
    }

    /// Whether `choice` is known to be read-only. It is refused here, at the start of the flow,
    /// with a toast; the picker stays open for another cluster.
    pub(super) fn refuse_read_only(&self, choice: &ClusterChoice, cx: &mut App) -> bool {
        if choice.read_only {
            self.toast(
                Toast::warning(format!(
                    "{} is read-only: nothing can be copied there. Turn read-only mode off first.",
                    choice.name
                ))
                .key("copy-read-only"),
                cx,
            );
        }
        choice.read_only
    }

    /// The cluster step is done: ask for the namespace, or (a cluster-scoped kind) go straight to
    /// the review.
    pub(super) fn cluster_chosen(
        &self,
        mut draft: CopyDraft,
        choice: ClusterChoice,
        window: &mut Window,
        cx: &mut App,
    ) {
        draft.cluster = Some(choice);
        if draft.source.namespace().is_none() {
            self.review(draft, None, window, cx);
            return;
        }
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let flow = self.clone();
        workspace.update(cx, |workspace, cx| {
            let picker = cx.new(|cx| NamespacePickerDelegate::picker(flow, draft, window, cx));
            workspace.show_modal(picker.clone(), window, cx);
            picker.update(cx, |picker, cx| picker.focus(window, cx));
        });
    }

    /// The namespace step is done: the review dialog.
    pub(super) fn review(
        &self,
        draft: CopyDraft,
        namespace: Option<String>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (Some(workspace), Some(cluster)) = (self.workspace.upgrade(), draft.cluster) else {
            return;
        };
        let (service, weak) = (self.service.clone(), self.workspace.clone());
        workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| {
                CopyDialog::new(
                    draft.source,
                    cluster.id,
                    cluster.name,
                    namespace,
                    service,
                    weak,
                    window,
                    cx,
                )
            });
            workspace.show_modal(dialog.clone(), window, cx);
            dialog.update(cx, |dialog, cx| dialog.focus(window, cx));
        });
    }

    pub(super) fn toast(&self, toast: Toast, cx: &mut App) {
        self.workspace
            .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
            .ok();
    }

    pub(super) fn service(&self) -> &CopyService {
        &self.service
    }
}
