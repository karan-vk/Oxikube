//! Drawing the label editor part of the bulk dialog: the labels / annotations switch, the key and
//! value rows, and the preview of one sample object.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, px,
};
use oxikube_domain::selection::{LabelField, LabelPreview};
use oxikube_ui::button::Button;
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::dialog::BulkDialog;

/// How many lines of the label preview show before the rest are counted.
const MAX_PREVIEW_LINES: usize = 10;

impl BulkDialog {
    pub(super) fn field_switch(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.editor.as_ref().map(|e| e.field).unwrap_or_default();
        let tokens = cx.colors();
        h_flex()
            .debug_selector(|| "bulk-field-switch".into())
            .gap(u(px(4.)))
            .items_center()
            .child(div().text_color(tokens.text_muted).child("Edit"))
            .children(
                [
                    (LabelField::Labels, "Labels"),
                    (LabelField::Annotations, "Annotations"),
                ]
                .map(|(field, label)| {
                    Button::new(label)
                        .label(label)
                        .small()
                        .selected(field == current)
                        .on_click(cx.listener(move |this, _, _, cx| this.set_field(field, cx)))
                }),
            )
    }

    pub(super) fn editor_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = &self.editor else {
            return div().into_any_element();
        };
        let tokens = cx.colors();
        let rows = editor.rows.iter().enumerate().map(|(ix, row)| {
            let removes = row.remove;
            h_flex()
                .debug_selector(move || format!("bulk-row-{ix}"))
                .gap(u(px(6.)))
                .items_center()
                .child(div().flex_1().min_w_0().child(Input::new(&row.key)))
                .child(
                    div()
                        .flex_none()
                        .text_color(tokens.text_muted)
                        .child(if removes { "removed" } else { "=" }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .opacity(if removes { 0.4 } else { 1. })
                        .child(Input::new(&row.value)),
                )
                .child(
                    Button::new(("bulk-remove", ix))
                        .label(if removes { "Set" } else { "Remove" })
                        .small()
                        .selected(removes)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(editor) = this.editor.as_mut() {
                                editor.toggle_remove(ix);
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(("bulk-drop", ix))
                        .icon(Icon::new(IconName::X))
                        .small()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(editor) = this.editor.as_mut() {
                                editor.drop_row(ix);
                            }
                            cx.notify();
                        })),
                )
        });
        v_flex()
            .gap(u(px(6.)))
            .children(rows)
            .child(
                h_flex().child(Button::new("bulk-add-row").label("Add").small().on_click(
                    cx.listener(|this, _, window, cx| {
                        if let Some(mut editor) = this.editor.take() {
                            editor.add_row(window, cx);
                            this.editor = Some(editor);
                        }
                        cx.notify();
                    }),
                )),
            )
            .into_any_element()
    }

    /// The sample object's labels after the edits: changes in colour, the rest muted, capped.
    pub(super) fn preview(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editor = self.editor.as_ref()?;
        let lines = editor.preview(cx);
        if lines.iter().all(|line| !line.is_change()) {
            return None;
        }
        let colors = cx.colors();
        let hidden = lines.len().saturating_sub(MAX_PREVIEW_LINES);
        // Changes first: they are what the user is checking.
        let mut ordered: Vec<&LabelPreview> = lines.iter().collect();
        ordered.sort_by_key(|line| !line.is_change());
        let rows = ordered.into_iter().take(MAX_PREVIEW_LINES).map(|line| {
            let (mark, text, tone) = match line {
                LabelPreview::Kept { key, value } => {
                    ("  ", format!("{key}={value}"), colors.text_muted)
                }
                LabelPreview::Added { key, value } => {
                    ("+ ", format!("{key}={value}"), colors.success)
                }
                LabelPreview::Changed { key, from, to } => {
                    ("~ ", format!("{key}={from} -> {to}"), colors.warning)
                }
                LabelPreview::Removed { key, value } => {
                    ("- ", format!("{key}={value}"), colors.error)
                }
            };
            div()
                .text_size(u(px(12.)))
                .text_color(tone)
                .child(format!("{mark}{text}"))
        });
        Some(
            v_flex()
                .debug_selector(|| "bulk-preview".into())
                .gap(u(px(2.)))
                .child(
                    div()
                        .text_color(colors.text_muted)
                        .child(format!("{} would have:", editor.sample_name)),
                )
                .children(rows)
                .children((hidden > 0).then(|| {
                    div()
                        .text_size(u(px(12.)))
                        .text_color(colors.text_muted)
                        .child(format!("…and {hidden} more"))
                }))
                .into_any_element(),
        )
    }
}
