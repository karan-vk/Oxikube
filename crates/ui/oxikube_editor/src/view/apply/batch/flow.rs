//! The batch flow on [`ManifestEditor`]: the preview, the dialog, the apply, the results.

use std::sync::Arc;

use gpui::{AppContext as _, AsyncWindowContext, Context, WeakEntity, Window};
use oxikube_app::{
    ApplyError, ApplyRequest, ApplyService, BatchPreview, BatchReport, DocumentStatus,
    PreviewOutcome,
};
use oxikube_domain::FieldCause;
use oxikube_domain::ids::ClusterId;
use oxikube_workspace::Toast;

use super::results::ResultsPanel;
use super::{BatchPreviewDialog, BatchSession};
use crate::apply::{Document, place_causes_in, preview_diff};
use crate::diff::ManifestDiff;
use crate::view::ManifestEditor;
use crate::view::apply::ApplyStage;
use crate::view::notice::NoticeLevel;

/// Where the documents go, from the editor.
pub(in crate::view::apply) struct Target {
    pub(in crate::view::apply) cluster: ClusterId,
    pub(in crate::view::apply) namespace: Option<String>,
    pub(in crate::view::apply) manager: String,
}

/// The diffs of a preview: for each document the server rehearsed, the live object against the
/// object as it would be stored; `None` for the others.
type Diffs = Vec<Option<Arc<ManifestDiff>>>;

/// The dry run of `documents` (from the buffer `text` at `version`), then the combined preview.
/// Runs inside the editor's apply task: the dry runs on the Kubernetes runtime, the diffs on the
/// background executor.
pub(in crate::view::apply) async fn run_preview(
    this: WeakEntity<ManifestEditor>,
    cx: &mut AsyncWindowContext,
    service: ApplyService,
    target: Target,
    documents: Vec<Document>,
    text: Arc<str>,
    version: u64,
) {
    let session = BatchSession {
        text,
        ranges: documents.iter().map(|d| d.range.clone()).collect(),
        version,
    };
    let requests: Vec<ApplyRequest> = documents
        .into_iter()
        .map(|document| {
            ApplyRequest::new(target.cluster.clone(), document.text)
                .in_namespace(target.namespace.clone())
                .managed_by(Some(target.manager.clone()))
        })
        .collect();
    let answer =
        oxikube_runtime::spawn_kube(cx, async move { service.preview_batch(requests).await }).await;
    let preview = match answer {
        Ok(preview) => preview,
        Err(error) => {
            let _ = this.update(cx, |view, cx| view.apply_refused(error.to_string(), cx));
            return;
        }
    };
    let (preview, diffs) = cx
        .background_spawn(async move {
            let diffs: Diffs = preview
                .documents
                .iter()
                .map(|document| document.preview().map(|p| Arc::new(preview_diff(p))))
                .collect();
            (preview, diffs)
        })
        .await;
    let _ = this.update_in(cx, |view, window, cx| {
        view.review_batch(preview, diffs, session, window, cx);
    });
}

impl ManifestEditor {
    /// The dry runs are in: the combined preview opens, and it is the confirmation. The server's
    /// causes for the documents it refused are put on those documents meanwhile.
    fn review_batch(
        &mut self,
        preview: BatchPreview,
        diffs: Diffs,
        session: BatchSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.notice = None;
        let Some(workspace) = self.apply.workspace.as_ref().and_then(|w| w.upgrade()) else {
            self.apply_refused(
                "The editor has no workspace to show the preview in".into(),
                cx,
            );
            return;
        };
        // A read-only cluster refuses every document: say so, as for one document.
        let read_only = preview.documents.iter().find_map(|d| match &d.outcome {
            PreviewOutcome::Failed(error @ ApplyError::ReadOnly { .. }) => Some(error),
            _ => None,
        });
        if let Some(error) = read_only
            && preview.applying() == 0
        {
            self.apply_refused(error.to_string(), cx);
            return;
        }
        let refused: Vec<(usize, &ApplyError)> = preview
            .documents
            .iter()
            .filter_map(|d| match &d.outcome {
                PreviewOutcome::Failed(error) => Some((d.index, error)),
                _ => None,
            })
            .collect();
        self.place_batch_causes(&refused, &session, window, cx);
        let editor = cx.entity().downgrade();
        let dialog = cx.new(|cx| BatchPreviewDialog::new(preview, diffs, session, editor, cx));
        self.show_review(&workspace, dialog, window, cx);
        cx.notify();
    }

    /// The preview's Apply: sends every document in order, unless the buffer changed since.
    pub(super) fn confirm_batch(
        &mut self,
        preview: BatchPreview,
        session: BatchSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.apply.service.clone() else {
            return;
        };
        if self.editor.read(cx).version() != session.version {
            self.apply_refused(
                "The manifest changed since the preview: apply again".into(),
                cx,
            );
            return;
        }
        self.apply.stage = ApplyStage::Applying;
        self.apply_notice(
            NoticeLevel::Info,
            format!("Applying {} documents…", preview.applying()),
            cx,
        );
        self.apply.task = Some(cx.spawn_in(window, async move |this, cx| {
            let answer =
                oxikube_runtime::spawn_kube(cx, async move { service.apply_batch(&preview).await })
                    .await;
            let _ = this.update_in(cx, |view, window, cx| match answer {
                Ok(report) => view.batch_applied(report, session, window, cx),
                Err(error) => view.apply_refused(error.to_string(), cx),
            });
        }));
        cx.notify();
    }

    /// Every document has been tried: the results panel under the buffer, a toast, and the
    /// buffer is clean when every document went through.
    fn batch_applied(
        &mut self,
        report: BatchReport,
        session: BatchSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply.stage = ApplyStage::Idle;
        let failures = report.failures();
        let summary = report.summary();
        let (level, toast) = if failures == 0 {
            (NoticeLevel::Info, Toast::success(summary.clone()))
        } else {
            (NoticeLevel::Warning, Toast::warning(summary.clone()))
        };
        self.apply_notice(level, summary, cx);
        if let Some(workspace) = self.apply.workspace.as_ref().and_then(|w| w.upgrade()) {
            workspace.update(cx, |workspace, cx| {
                workspace.show_toast(toast, cx);
            });
        }
        let failed: Vec<(usize, &ApplyError)> = report
            .documents
            .iter()
            .filter_map(|d| match &d.status {
                DocumentStatus::Failed(error) => Some((d.index, error)),
                _ => None,
            })
            .collect();
        self.place_batch_causes(&failed, &session, window, cx);
        if failures == 0 && self.editor.read(cx).version() == session.version {
            self.clean_version = session.version;
            cx.emit(oxikube_workspace::ItemEvent::UpdateTab);
        }
        self.apply.results = Some(ResultsPanel::new(report, session));
        cx.notify();
    }

    /// Puts the server's causes for the refused documents on their fields, off the UI thread.
    fn place_batch_causes(
        &mut self,
        refused: &[(usize, &ApplyError)],
        session: &BatchSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let causes: Vec<(std::ops::Range<usize>, Vec<FieldCause>)> = refused
            .iter()
            .filter_map(|(index, error)| match error {
                ApplyError::Invalid { causes, .. } if !causes.is_empty() => {
                    Some((session.ranges.get(*index)?.clone(), causes.clone()))
                }
                _ => None,
            })
            .collect();
        if causes.is_empty() {
            return;
        }
        let (text, version) = (session.text.clone(), session.version);
        // Detached, not stored: it starts from inside the step that finished. It holds the
        // editor weakly, and a result for an older buffer is dropped by `show_server`.
        cx.spawn_in(window, async move |this, cx| {
            let diagnostics = cx
                .background_spawn(async move {
                    let mut all = Vec::new();
                    for (range, causes) in &causes {
                        all.extend(place_causes_in(&text, range, causes).diagnostics);
                    }
                    all.sort_by_key(|d| d.range.start);
                    all
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                let model = &mut view.model;
                view.editor.update(cx, |editor, cx| {
                    model.show_server(&mut editor.api(window, cx), version, diagnostics);
                });
                cx.notify();
            });
        })
        .detach();
    }
}
