//! The notice bar: one line under the toolbar that says what a conversion did (and could not
//! carry over), with an Undo button while the conversion is the last edit.
//!
//! A notice goes away with the next edit of the buffer (an undo included), or when dismissed.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    div,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::manifest_editor::ManifestEditor;

/// How serious a [`Notice`] is: its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    /// Something happened as asked.
    Info,
    /// Something was lost on the way, or needs a second look.
    Warning,
    /// The action was refused; nothing changed.
    Error,
}

/// What the notice bar shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// Colour of the line.
    pub level: NoticeLevel,
    /// The line itself.
    pub text: SharedString,
    /// Whether the bar offers Undo (the last edit was a conversion).
    pub undoable: bool,
}

impl Notice {
    pub(super) fn new(level: NoticeLevel, text: impl Into<SharedString>) -> Self {
        Self {
            level,
            text: text.into(),
            undoable: false,
        }
    }

    pub(super) fn undoable(mut self) -> Self {
        self.undoable = true;
        self
    }
}

impl ManifestEditor {
    /// The notice bar, when there is a notice.
    pub(super) fn notice_bar(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let notice = self.notice.clone()?;
        let tokens = cx.tokens();
        let color = match notice.level {
            NoticeLevel::Info => tokens.colors.text_muted,
            NoticeLevel::Warning => tokens.colors.warning,
            NoticeLevel::Error => tokens.colors.error,
        };
        let undo = notice.undoable.then(|| {
            Button::new("manifest-notice-undo")
                .label("Undo")
                .ghost()
                .xsmall()
                .on_click(cx.listener(|view, _, window, cx| {
                    let editor = view.editor.clone();
                    editor.update(cx, |editor, cx| editor.undo(window, cx));
                }))
        });
        Some(
            h_flex()
                .id("manifest-notice")
                .debug_selector(|| "manifest-notice".into())
                .flex_none()
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.xs))
                .items_center()
                .border_b_1()
                .border_color(tokens.colors.border_variant)
                .text_size(u(tokens.font.small))
                .text_color(color)
                .child(div().flex_1().min_w_0().truncate().child(notice.text))
                .children(undo)
                .child(
                    Button::new("manifest-notice-dismiss")
                        .label("Dismiss")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.notice = None;
                            cx.notify();
                        })),
                ),
        )
    }
}
