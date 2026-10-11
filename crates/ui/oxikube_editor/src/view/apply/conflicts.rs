//! [`ConflictDialog`]: the fields of the manifest that other field managers own, and the choice
//! to take them over.
//!
//! A server-side apply refuses to change a field another manager set (409, one cause per field).
//! The dialog lists each field with its manager and when that manager last wrote the object.
//! Force previews the apply again with `force`, so the user still sees the diff before anything
//! is written; Cancel leaves everything as it is (or the user drops those fields and applies
//! again). The list is a `uniform_list`.

use std::ops::Range;

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, WeakEntity, Window, div,
    px, uniform_list,
};
use oxikube_app::FieldConflict;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, u};
use oxikube_workspace::modal::{DIALOG_KEY_CONTEXT, ModalPlacement, ModalView};

use crate::view::ManifestEditor;

/// A row's height, before the UI zoom.
const ROW_HEIGHT: f32 = 22.;
/// Most rows shown before the list scrolls.
const VISIBLE_ROWS: f32 = 10.;

/// The conflicts of one apply. See the [module docs](self).
pub struct ConflictDialog {
    conflicts: Vec<FieldConflict>,
    editor: WeakEntity<ManifestEditor>,
    /// Whether Force was chosen (the editor then previews again).
    forced: bool,
    focus: FocusHandle,
}

impl EventEmitter<DismissEvent> for ConflictDialog {}

impl Focusable for ConflictDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for ConflictDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    fn on_before_dismiss(&mut self, _: &mut Window, cx: &mut App) -> bool {
        if !self.forced {
            self.editor
                .update(cx, |editor, cx| editor.apply_cancelled(cx))
                .ok();
        }
        true
    }
}

impl ConflictDialog {
    /// The list of `conflicts` of `editor`'s apply.
    pub fn new(
        conflicts: Vec<FieldConflict>,
        editor: WeakEntity<ManifestEditor>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            conflicts,
            editor,
            forced: false,
            focus: cx.focus_handle(),
        }
    }

    /// The conflicts listed.
    pub fn conflicts(&self) -> &[FieldConflict] {
        &self.conflicts
    }

    /// The managers named, in first-seen order.
    pub fn managers(&self) -> Vec<&str> {
        oxikube_app::apply::conflict_managers(&self.conflicts)
    }

    /// Force: preview the apply again, taking the fields over.
    pub fn force(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.forced {
            return;
        }
        self.forced = true;
        // The editor is idle again before it starts over (the dialog closes after this).
        self.editor
            .update(cx, |editor, cx| {
                editor.apply_cancelled(cx);
                editor.start_apply(true, window, cx);
            })
            .ok();
        cx.emit(DismissEvent);
    }

    /// Cancel: nothing is sent.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn row(conflict: &FieldConflict, cx: &App) -> gpui::AnyElement {
        let colors = cx.tokens().colors;
        let field = conflict.field.trim_start_matches('.').to_owned();
        let manager = conflict
            .manager
            .clone()
            .unwrap_or_else(|| "another manager".to_owned());
        let when = conflict
            .applied_at
            .as_deref()
            .map_or_else(String::new, |time| format!("last applied {time}"));
        h_flex()
            .h(u(px(ROW_HEIGHT)))
            .w_full()
            .gap(u(px(12.)))
            .overflow_hidden()
            .whitespace_nowrap()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(cx.mono_font_family())
                    .child(SharedString::from(field)),
            )
            .child(div().flex_none().w(u(px(140.))).truncate().child(manager))
            .child(
                div()
                    .flex_none()
                    .w(u(px(220.)))
                    .text_color(colors.text_muted)
                    .child(when),
            )
            .into_any_element()
    }
}

impl Render for ConflictDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let rows = self.conflicts.len();
        let height = u(px(ROW_HEIGHT * VISIBLE_ROWS.min(rows.max(1) as f32)));
        let managers = self.managers().join(", ");
        let message = format!(
            "{rows} field(s) of the manifest are managed by {}. Force takes them over (the \
             managers may set them back); Cancel leaves the object as it is.",
            if managers.is_empty() {
                "another manager"
            } else {
                managers.as_str()
            }
        );
        v_flex()
            .id("apply-conflicts")
            .debug_selector(|| "apply-conflicts".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(640.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.force(window, cx)))
            .child(
                DialogHeader::new()
                    .child(DialogTitle::new().child("Fields owned by other managers")),
            )
            .child(
                div()
                    .debug_selector(|| "apply-conflicts-message".into())
                    .text_color(colors.text_muted)
                    .child(message),
            )
            .child(
                div()
                    .debug_selector(|| "apply-conflicts-list".into())
                    .h(height)
                    .w_full()
                    .child(
                        uniform_list(
                            "apply-conflict-rows",
                            rows,
                            cx.processor(|this, range: Range<usize>, _, cx| {
                                range
                                    .map(|ix| Self::row(&this.conflicts[ix], cx))
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .size_full(),
                    ),
            )
            .child(
                DialogFooter::new()
                    .child(
                        div()
                            .debug_selector(|| "apply-conflicts-cancel".into())
                            .child(
                                Button::new("apply-conflicts-cancel")
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "apply-conflicts-force".into())
                            .child(
                                Button::new("apply-conflicts-force")
                                    .label("Force")
                                    .danger()
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.force(window, cx)),
                                    ),
                            ),
                    ),
            )
    }
}
