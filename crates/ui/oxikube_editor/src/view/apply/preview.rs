//! [`ApplyPreviewDialog`]: the server's dry run as a diff, and the confirmation of the apply.
//!
//! The diff is the live object against the object as the server would store it (defaults and
//! admission included), normalised like the diff vs live. Its rows are a `uniform_list`: a large
//! object costs one screen of rows. Enter applies, Escape (or a click outside) cancels; nothing
//! is written until Apply.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, UniformListScrollHandle,
    WeakEntity, Window, div, px, uniform_list,
};
use oxikube_app::{ApplyMethod, ApplyPreview};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::layout::v_flex;
use oxikube_ui::{ActiveTokens as _, u};
use oxikube_workspace::modal::{DIALOG_KEY_CONTEXT, ModalPlacement, ModalView};

use crate::diff::ManifestDiff;
use crate::view::ManifestEditor;
use crate::view::live::{ROW_HEIGHT, inline_rows, summary};

/// Most diff rows shown before the list scrolls.
const VISIBLE_ROWS: f32 = 18.0;

/// The preview of one apply. See the [module docs](self).
pub struct ApplyPreviewDialog {
    preview: ApplyPreview,
    diff: Arc<ManifestDiff>,
    editor: WeakEntity<ManifestEditor>,
    /// The buffer version the dry run was made from.
    version: u64,
    /// Whether Apply was chosen (the editor then owns what happens next).
    confirmed: bool,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl EventEmitter<DismissEvent> for ApplyPreviewDialog {}

impl Focusable for ApplyPreviewDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for ApplyPreviewDialog {
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

impl ApplyPreviewDialog {
    /// The preview of `preview` with its `diff`, for `editor`'s buffer at `version`.
    pub fn new(
        preview: ApplyPreview,
        diff: Arc<ManifestDiff>,
        editor: WeakEntity<ManifestEditor>,
        version: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            preview,
            diff,
            editor,
            version,
            confirmed: false,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    /// What it previews.
    pub fn preview(&self) -> &ApplyPreview {
        &self.preview
    }

    /// The diff it shows.
    pub fn diff(&self) -> &ManifestDiff {
        &self.diff
    }

    /// "Apply ConfigMap default/web to kind-oxikube?".
    pub fn title(&self) -> SharedString {
        let verb = if self.preview.creates() {
            "Create"
        } else {
            "Apply"
        };
        format!("{verb} {}?", self.preview.label()).into()
    }

    /// The line under the title: what changes, and how it will be sent.
    pub fn description(&self) -> String {
        let what = if self.preview.creates() {
            "The server dry run creates the object".to_owned()
        } else if self.diff.is_empty() {
            "The server dry run changes nothing".to_owned()
        } else {
            format!("Server dry run: {}", summary(&self.diff))
        };
        let how = match self.preview.method {
            ApplyMethod::ServerSide => {
                let manager = self.manager();
                format!("Server-side apply as {manager}")
            }
            ApplyMethod::Replace => format!(
                "The server cannot server-side apply this kind: it is replaced at resourceVersion {}",
                self.preview.resource_version.as_deref().unwrap_or("?")
            ),
            ApplyMethod::Create => {
                "The server cannot server-side apply this kind: it is created".to_owned()
            }
        };
        let force = if self.preview.request.force {
            ", taking over the fields other managers own"
        } else {
            ""
        };
        format!("{what}. {how}{force}.")
    }

    fn manager(&self) -> &str {
        self.preview
            .request
            .field_manager
            .as_deref()
            .unwrap_or(oxikube_app::apply::DEFAULT_FIELD_MANAGER)
    }

    /// Apply: hands the preview back to the editor, which sends it.
    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirmed {
            return;
        }
        self.confirmed = true;
        let preview = self.preview.clone();
        let version = self.version;
        self.editor
            .update(cx, |editor, cx| {
                editor.confirm_apply(preview, version, window, cx);
            })
            .ok();
        cx.emit(DismissEvent);
    }

    /// Cancel: nothing is sent.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn rows(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tokens = cx.tokens();
        let rows = self.diff.inline_rows().len();
        let height = u(px(ROW_HEIGHT * VISIBLE_ROWS.min(rows.max(1) as f32)));
        let diff = self.diff.clone();
        div()
            .debug_selector(|| "apply-preview-diff".into())
            .h(height)
            .w_full()
            .border_1()
            .border_color(tokens.colors.border_variant)
            .rounded(u(tokens.radius.sm))
            .overflow_hidden()
            .font_family(cx.mono_font_family())
            .text_size(u(tokens.font.mono))
            .child(
                uniform_list(
                    "apply-preview-rows",
                    rows,
                    cx.processor(move |_, range: Range<usize>, _, cx| {
                        inline_rows(&diff, range, cx)
                    }),
                )
                .track_scroll(&self.scroll)
                .size_full(),
            )
    }
}

impl Render for ApplyPreviewDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let mut apply = Button::new("apply-preview-confirm").label(if self.preview.creates() {
            "Create"
        } else {
            "Apply"
        });
        apply = if self.preview.request.force {
            apply.danger()
        } else {
            apply.primary()
        };
        v_flex()
            .id("apply-preview")
            .debug_selector(|| "apply-preview".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(760.)))
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
            .child(DialogHeader::new().child(DialogTitle::new().child(self.title())))
            .child(
                div()
                    .debug_selector(|| "apply-preview-description".into())
                    .text_color(colors.text_muted)
                    .child(self.description()),
            )
            .child(self.rows(cx))
            .child(
                DialogFooter::new()
                    .child(
                        div()
                            .debug_selector(|| "apply-preview-cancel".into())
                            .child(
                                Button::new("apply-preview-cancel")
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "apply-preview-confirm".into())
                            .child(apply.on_click(
                                cx.listener(|this, _, window, cx| this.confirm(window, cx)),
                            )),
                    ),
            )
    }
}
