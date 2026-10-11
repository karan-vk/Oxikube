//! The apply flow on [`ManifestEditor`]: checks, dry run, preview, apply, and what each answer
//! does to the editor. See the [module docs](super).

use std::sync::Arc;

use gpui::{AppContext as _, Context, Window};
use oxikube_app::{ApplyError, ApplyPreview, ApplyReport, ApplyRequest, FieldConflict};
use oxikube_ui::editor::DiagnosticLevel;
use oxikube_workspace::Toast;

use super::{ApplyPreviewDialog, ApplyStage, ConflictDialog};
use crate::apply::{place_causes, preview_diff};
use crate::open::{MASKED_CODE, ResourceBinding};
use crate::view::ManifestEditor;
use crate::view::notice::{Notice, NoticeLevel};
use crate::view::validation::validate_text_in;

impl ManifestEditor {
    /// Applies the buffer (`resource::Apply`: the key, the palette, the toolbar): the buffer's own
    /// checks, the server's dry run, then the preview to confirm. One apply at a time; nothing
    /// without a cluster, and a buffer with errors (a masked Secret value among them) is refused.
    pub fn begin_apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_apply(false, window, cx);
    }

    /// [`begin_apply`](Self::begin_apply), taking over the fields other managers own when
    /// `force` (the conflict dialog's Force).
    pub(super) fn start_apply(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.apply.stage != ApplyStage::Idle {
            return;
        }
        let (Some(service), Some(cluster)) = (self.apply.service.clone(), self.cluster().cloned())
        else {
            self.apply_notice(
                NoticeLevel::Error,
                "No cluster: open the manifest from a cluster tab to apply it".into(),
                cx,
            );
            return;
        };
        if oxikube_runtime::mode(cx).is_none() {
            self.apply_notice(
                NoticeLevel::Error,
                "The cluster is not connected".into(),
                cx,
            );
            return;
        }
        let snapshot = self.editor.read(cx).snapshot(cx);
        let version = snapshot.version();
        let known = self.model.known().clone();
        let mode = self.model.mode();
        let masking = self.binding.as_ref().is_some_and(ResourceBinding::masking);
        let settings = self.settings(cx);
        let validating = settings.validation;
        let manager = settings.field_manager.clone();
        let namespace = self
            .binding
            .as_ref()
            .and_then(|binding| binding.target().namespace().map(str::to_owned));
        self.apply.stage = ApplyStage::DryRun;
        self.apply_notice(NoticeLevel::Info, "Dry run on the server…".into(), cx);
        self.apply.task = Some(cx.spawn_in(window, async move |this, cx| {
            // The checks of this very text: a result shown for an older one does not count.
            let (text, errors) = cx
                .background_spawn(async move {
                    let text = snapshot.to_text();
                    let check = validate_text_in(version, text.clone(), &known, mode, masking);
                    let errors = check
                        .diagnostics
                        .iter()
                        .filter(|d| d.level == DiagnosticLevel::Error)
                        .filter(|d| validating || d.code.as_deref() == Some(MASKED_CODE))
                        .count();
                    (text, errors)
                })
                .await;
            if errors > 0 {
                let _ = this.update(cx, |view, cx| {
                    let noun = if errors == 1 { "error" } else { "errors" };
                    view.apply_refused(format!("Fix the {errors} {noun} before applying"), cx);
                });
                return;
            }
            let mut request = ApplyRequest::new(cluster, text.to_string())
                .in_namespace(namespace)
                .managed_by(Some(manager));
            if force {
                request = request.forced();
            }
            let answer =
                oxikube_runtime::spawn_kube(cx, async move { service.preview(request).await })
                    .await
                    .unwrap_or_else(|error| Err(ApplyError::Failed(error.to_string())));
            let preview = match answer {
                Ok(preview) => preview,
                Err(error) => {
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.apply_failed(error, version, text, window, cx);
                    });
                    return;
                }
            };
            let (preview, diff) = cx
                .background_spawn(async move {
                    let diff = Arc::new(preview_diff(&preview));
                    (preview, diff)
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.review(preview, diff, version, window, cx);
            });
        }));
        cx.notify();
    }

    /// The dry run passed: the preview opens, and it is the confirmation.
    fn review(
        &mut self,
        preview: ApplyPreview,
        diff: Arc<crate::diff::ManifestDiff>,
        version: u64,
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
        self.apply.stage = ApplyStage::Reviewing;
        let editor = cx.entity().downgrade();
        workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| ApplyPreviewDialog::new(preview, diff, editor, version, cx));
            workspace.show_modal(dialog, window, cx);
        });
        cx.notify();
    }

    /// The preview's Apply: sends what it showed, unless the buffer changed since.
    pub(super) fn confirm_apply(
        &mut self,
        preview: ApplyPreview,
        version: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.apply.service.clone() else {
            return;
        };
        if self.editor.read(cx).version() != version {
            self.apply_refused(
                "The manifest changed since the preview: apply again".into(),
                cx,
            );
            return;
        }
        self.apply.stage = ApplyStage::Applying;
        self.apply_notice(
            NoticeLevel::Info,
            format!("Applying {}…", preview.label()),
            cx,
        );
        let text: Arc<str> = Arc::from(preview.request.manifest.as_str());
        self.apply.task = Some(cx.spawn_in(window, async move |this, cx| {
            let answer =
                oxikube_runtime::spawn_kube(cx, async move { service.apply(&preview).await })
                    .await
                    .unwrap_or_else(|error| Err(ApplyError::Failed(error.to_string())));
            let _ = this.update_in(cx, |view, window, cx| match answer {
                Ok(report) => view.applied(report, version, cx),
                Err(error) => view.apply_failed(error, version, text, window, cx),
            });
        }));
        cx.notify();
    }

    /// The preview or the conflict list was closed without applying.
    pub(super) fn apply_cancelled(&mut self, cx: &mut Context<Self>) {
        if self.apply.stage == ApplyStage::Reviewing {
            self.apply.stage = ApplyStage::Idle;
            cx.notify();
        }
    }

    /// The server took it: a toast, the notice, and the buffer is clean against what it holds.
    fn applied(&mut self, report: ApplyReport, version: u64, cx: &mut Context<Self>) {
        self.apply.stage = ApplyStage::Idle;
        let level = if report.method.is_fallback() {
            NoticeLevel::Warning
        } else {
            NoticeLevel::Info
        };
        self.apply_notice(level, report.message.clone(), cx);
        if let Some(workspace) = self.apply.workspace.as_ref().and_then(|w| w.upgrade()) {
            let toast = if report.method.is_fallback() {
                Toast::warning(report.message.clone())
            } else {
                Toast::success(report.message.clone())
            };
            workspace.update(cx, |workspace, cx| {
                workspace.show_toast(toast, cx);
            });
        }
        if self.editor.read(cx).version() == version {
            self.clean_version = version;
            let applied_version = report
                .object
                .pointer("/metadata/resourceVersion")
                .and_then(|v| v.as_str());
            self.live.sync.applied(applied_version);
            cx.emit(oxikube_workspace::ItemEvent::UpdateTab);
        }
        cx.notify();
    }

    /// The dry run or the apply failed; `text` is what was sent (the buffer at `version`).
    fn apply_failed(
        &mut self,
        error: ApplyError,
        version: u64,
        text: Arc<str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply.stage = ApplyStage::Idle;
        match error {
            ApplyError::Conflicts(conflicts) => self.show_conflicts(conflicts, window, cx),
            ApplyError::Invalid { message, causes } if !causes.is_empty() => {
                self.apply_notice(
                    NoticeLevel::Error,
                    format!("The server rejected the manifest: {message}"),
                    cx,
                );
                self.place_server_causes(text, causes, version, window, cx);
            }
            ApplyError::Stale(_) => self.apply_notice(
                NoticeLevel::Warning,
                "The object changed on the server since the preview: apply again to review it"
                    .into(),
                cx,
            ),
            other => self.apply_notice(NoticeLevel::Error, other.to_string(), cx),
        }
        cx.notify();
    }

    /// Opens the conflict list; its Force previews the apply again with force.
    fn show_conflicts(
        &mut self,
        conflicts: Vec<FieldConflict>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = ApplyError::Conflicts(conflicts.clone()).to_string();
        self.apply_notice(NoticeLevel::Warning, message, cx);
        let Some(workspace) = self.apply.workspace.as_ref().and_then(|w| w.upgrade()) else {
            return;
        };
        self.apply.stage = ApplyStage::Reviewing;
        let editor = cx.entity().downgrade();
        workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| ConflictDialog::new(conflicts, editor, cx));
            workspace.show_modal(dialog, window, cx);
        });
    }

    /// Puts the server's causes on their fields, off the UI thread; the ones without a place join
    /// the notice.
    fn place_server_causes(
        &mut self,
        text: Arc<str>,
        causes: Vec<oxikube_domain::FieldCause>,
        version: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Detached, not stored: it starts from inside the step that failed (a stored task never
        // replaces itself). It holds the editor weakly, and a result for an older buffer is
        // dropped by `show_server`.
        cx.spawn_in(window, async move |this, cx| {
            let placed = cx
                .background_spawn(async move { place_causes(&text, &causes) })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                let model = &mut view.model;
                view.editor.update(cx, |editor, cx| {
                    model.show_server(&mut editor.api(window, cx), version, placed.diagnostics);
                });
                if !placed.unplaced.is_empty()
                    && let Some(notice) = view.notice.as_mut()
                {
                    notice.text =
                        format!("{} · {}", notice.text, placed.unplaced.join(" · ")).into();
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Refuses the apply with `text`; nothing was sent.
    fn apply_refused(&mut self, text: String, cx: &mut Context<Self>) {
        self.apply.stage = ApplyStage::Idle;
        self.apply_notice(NoticeLevel::Error, text, cx);
    }

    /// Shows `text` in the notice bar until the next edit.
    fn apply_notice(&mut self, level: NoticeLevel, text: String, cx: &mut Context<Self>) {
        self.set_notice(Notice::new(level, text), cx);
    }
}
