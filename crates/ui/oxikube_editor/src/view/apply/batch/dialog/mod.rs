//! [`BatchPreviewDialog`]: the combined preview of a multi-document apply, and its confirmation.
//!
//! One row per document with what the server's dry run says it would do (create, update, leave
//! unchanged, wait for an earlier document, or why it was refused), and under the list the diff of
//! the selected document. Both lists are `uniform_list`s: a buffer of hundreds of documents costs
//! one screen of rows. Enter applies (the documents the dry run accepted, in order), Escape cancels;
//! nothing is written until Apply.

use std::sync::Arc;

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString, Styled as _,
    UniformListScrollHandle, WeakEntity, Window, div, px,
};
use oxikube_app::{BatchPreview, PreviewOutcome};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::layout::{Disableable as _, v_flex};
use oxikube_ui::{ActiveTokens as _, u};
use oxikube_workspace::modal::{DIALOG_KEY_CONTEXT, ModalPlacement, ModalView};

use super::BatchSession;
use crate::diff::ManifestDiff;
use crate::view::ManifestEditor;
use crate::view::live::summary;

/// A document row's height, before the UI zoom.
pub(super) const ROW_HEIGHT: f32 = 24.;
/// Most document rows shown before the list scrolls.
pub(super) const VISIBLE_ROWS: f32 = 7.;
/// Most diff rows shown before the diff scrolls.
pub(super) const VISIBLE_DIFF_ROWS: f32 = 12.;

/// The preview of one multi-document apply. See the module docs.
pub struct BatchPreviewDialog {
    preview: BatchPreview,
    diffs: Vec<Option<Arc<ManifestDiff>>>,
    session: BatchSession,
    editor: WeakEntity<ManifestEditor>,
    selected: usize,
    /// Whether Apply was chosen (the editor then owns what happens next).
    confirmed: bool,
    list_scroll: UniformListScrollHandle,
    diff_scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

mod rows;

impl EventEmitter<DismissEvent> for BatchPreviewDialog {}

impl Focusable for BatchPreviewDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for BatchPreviewDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Closed without Apply (Escape, a click outside, Cancel): the editor's apply is over.
    fn on_before_dismiss(&mut self, _: &mut Window, cx: &mut App) -> bool {
        if !self.confirmed {
            self.editor
                .update(cx, |editor, cx| editor.apply_cancelled(cx))
                .ok();
        }
        true
    }
}

impl BatchPreviewDialog {
    /// The preview of `preview` (with `diffs`, one per document that has one) for `editor`'s
    /// buffer as `session` describes it.
    pub fn new(
        preview: BatchPreview,
        diffs: Vec<Option<Arc<ManifestDiff>>>,
        session: BatchSession,
        editor: WeakEntity<ManifestEditor>,
        cx: &mut Context<Self>,
    ) -> Self {
        // The first thing to look at: a document the server refused, else the first.
        let selected = preview
            .documents
            .iter()
            .position(|d| matches!(d.outcome, PreviewOutcome::Failed(_)))
            .unwrap_or(0);
        Self {
            preview,
            diffs,
            session,
            editor,
            selected,
            confirmed: false,
            list_scroll: UniformListScrollHandle::new(),
            diff_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    /// What it previews.
    pub fn preview(&self) -> &BatchPreview {
        &self.preview
    }

    /// The selected document's place, from 0.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// "Apply 4 documents to the cluster?".
    pub fn title(&self) -> SharedString {
        format!("Apply {} documents?", self.preview.documents.len()).into()
    }

    /// The line under the title: how many of each, and the order and failure policy.
    pub fn description(&self) -> String {
        let counts = self.preview.counts();
        let mut parts: Vec<String> = Vec::new();
        for (n, what) in [
            (counts.create, "to create"),
            (counts.update, "to change"),
            (counts.unchanged, "unchanged"),
            (counts.deferred, "after an earlier document"),
            (counts.failed, "refused by the dry run"),
            (counts.skipped, "skipped"),
        ] {
            if n > 0 {
                parts.push(format!("{n} {what}"));
            }
        }
        format!(
            "Server dry run: {}. Applied in buffer order; a failure does not stop the rest, as with kubectl apply.",
            parts.join(", ")
        )
    }

    /// Selects document `index` and shows its diff.
    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.preview.documents.len() && index != self.selected {
            self.selected = index;
            self.list_scroll
                .scroll_to_item(index, gpui::ScrollStrategy::Nearest);
            self.diff_scroll
                .scroll_to_item(0, gpui::ScrollStrategy::Top);
            cx.notify();
        }
    }

    /// Apply: hands the preview back to the editor, which sends the documents.
    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirmed || self.preview.applying() == 0 {
            return;
        }
        self.confirmed = true;
        let (preview, session) = (self.preview.clone(), self.session.clone());
        self.editor
            .update(cx, |editor, cx| {
                editor.confirm_batch(preview, session, window, cx);
            })
            .ok();
        cx.emit(DismissEvent);
    }

    /// Cancel: nothing is sent.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "down" => self.select(self.selected + 1, cx),
            "up" => self.select(self.selected.saturating_sub(1), cx),
            _ => {}
        }
    }
}

impl Render for BatchPreviewDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let applying = self.preview.applying();
        let diff_summary = self
            .diffs
            .get(self.selected)
            .and_then(|d| d.as_ref())
            .map(|d| summary(d))
            .unwrap_or_default();
        let apply = Button::new("batch-preview-confirm")
            .label(match applying {
                0 => "Nothing to apply".to_owned(),
                1 => "Apply 1 document".to_owned(),
                n => format!("Apply {n} documents"),
            })
            .primary()
            .disabled(applying == 0);
        v_flex()
            .id("batch-preview")
            .debug_selector(|| "batch-preview".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(820.)))
            .max_w(gpui::relative(0.9))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.confirm(window, cx)))
            .on_key_down(cx.listener(|this, event, _, cx| this.on_key(event, cx)))
            .child(DialogHeader::new().child(DialogTitle::new().child(self.title())))
            .child(
                div()
                    .debug_selector(|| "batch-preview-description".into())
                    .text_color(colors.text_muted)
                    .child(self.description()),
            )
            .child(self.list(cx))
            .child(
                div()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(diff_summary),
            )
            .child(self.body(cx))
            .child(
                DialogFooter::new()
                    .child(
                        div()
                            .debug_selector(|| "batch-preview-cancel".into())
                            .child(
                                Button::new("batch-preview-cancel")
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "batch-preview-confirm".into())
                            .child(apply.on_click(
                                cx.listener(|this, _, window, cx| this.confirm(window, cx)),
                            )),
                    ),
            )
    }
}
