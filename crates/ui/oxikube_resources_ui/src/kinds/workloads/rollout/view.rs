//! Drawing the set-image dialog: one row per container, the diff of old to new, the warnings.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_app::object_label;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, StyledExt as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dialog::{SetImageDialog, SetImageStage};

/// What the dialog says about the rollout the change starts.
pub(super) const ROLLOUT_NOTE: &str = "Changing an image rolls the workload: new pods start with the new images and the old ones are \
     replaced.";

impl SetImageDialog {
    fn title(&self) -> SharedString {
        format!("Set image of {}", self.target.name).into()
    }

    /// `Deployment shop/web`.
    fn subtitle(&self) -> SharedString {
        object_label(&self.target).into()
    }

    fn container_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.colors();
        let editing = self.stage == SetImageStage::Editing;
        self.fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let problem = self.problem(index, cx);
                v_flex()
                    .debug_selector(move || format!("setimage-row-{index}"))
                    .gap(u(px(4.)))
                    .child(
                        h_flex()
                            .gap(u(px(6.)))
                            .items_center()
                            .child(
                                div()
                                    .debug_selector(move || format!("setimage-name-{index}"))
                                    .font_semibold()
                                    .child(field.name.to_string()),
                            )
                            .children(field.init.then(|| {
                                div()
                                    .px(u(px(6.)))
                                    .rounded(u(px(4.)))
                                    .bg(colors.element)
                                    .text_size(u(px(11.)))
                                    .text_color(colors.text_muted)
                                    .child("init")
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(u(px(12.)))
                                    .text_color(colors.text_muted)
                                    .child(format!("now {}", field.current)),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(move || format!("setimage-input-{index}"))
                            .child(Input::new(&field.input).disabled(!editing)),
                    )
                    .children(problem.map(|message| {
                        div()
                            .debug_selector(move || format!("setimage-problem-{index}"))
                            .text_size(u(px(12.)))
                            .text_color(colors.error)
                            .child(message)
                    }))
                    .into_any_element()
            })
            .collect()
    }

    /// The diff: each changed container's old image struck against the new one.
    fn diff(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let edits = self.edits(cx);
        let body = if edits.is_empty() {
            div()
                .debug_selector(|| "setimage-no-changes".into())
                .text_color(colors.text_muted)
                .child("Edit an image to see what changes.")
                .into_any_element()
        } else {
            v_flex()
                .gap(u(px(2.)))
                .children(edits.iter().enumerate().map(|(index, edit)| {
                    h_flex()
                        .debug_selector(move || format!("setimage-diff-{index}"))
                        .gap(u(px(6.)))
                        .items_start()
                        .text_size(u(px(12.)))
                        .child(
                            div()
                                .flex_none()
                                .font_semibold()
                                .child(edit.container.to_string()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .text_color(colors.error)
                                .child(format!("- {}", edit.from)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .text_color(colors.success)
                                .child(format!("+ {}", edit.to)),
                        )
                }))
                .into_any_element()
        };
        v_flex()
            .debug_selector(|| "setimage-diff".into())
            .gap(u(px(4.)))
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(colors.text_muted)
                    .child("Changes"),
            )
            .child(body)
            .into_any_element()
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let note = |id: &'static str, color, text: String| {
            h_flex()
                .debug_selector(move || id.into())
                .gap(u(px(8.)))
                .items_start()
                .text_size(u(px(12.)))
                .text_color(color)
                .child(Icon::new(IconName::TriangleAlert).size(u(px(14.))))
                .child(div().flex_1().min_w_0().child(text))
                .into_any_element()
        };
        let content: AnyElement = match self.stage {
            SetImageStage::Loading => h_flex()
                .debug_selector(|| "setimage-loading".into())
                .gap(u(px(8.)))
                .items_center()
                .text_color(colors.text_muted)
                .child(
                    Spinner::new()
                        .icon(Icon::new(IconName::LoaderCircle))
                        .color(colors.accent),
                )
                .child("Reading the containers…")
                .into_any_element(),
            _ if self.fields.is_empty() => div()
                .debug_selector(|| "setimage-none".into())
                .text_color(colors.text_muted)
                .child("The containers could not be read.")
                .into_any_element(),
            _ => v_flex()
                .gap(u(px(12.)))
                .child(
                    v_flex()
                        .id("setimage-rows")
                        .gap(u(px(10.)))
                        .max_h(u(px(320.)))
                        .overflow_y_scroll()
                        .children(self.container_rows(cx)),
                )
                .child(self.diff(cx))
                .child(note(
                    "setimage-note",
                    colors.text_muted,
                    ROLLOUT_NOTE.to_owned(),
                ))
                .into_any_element(),
        };
        v_flex()
            .gap(u(px(12.)))
            .child(
                div()
                    .debug_selector(|| "setimage-subtitle".into())
                    .text_color(colors.text_muted)
                    .child(self.subtitle()),
            )
            .child(content)
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "setimage-error".into())
                    .text_color(colors.error)
                    .child(error)
            }))
            .into_any_element()
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let applying = self.stage == SetImageStage::Applying;
        let count = self.edits(cx).len();
        let label: SharedString = match count {
            0 => "Set image".into(),
            1 => "Set 1 image".into(),
            n => format!("Set {n} images").into(),
        };
        DialogFooter::new()
            .child(
                Button::new("setimage-cancel")
                    .label("Cancel")
                    .disabled(applying)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
            .child(
                div().debug_selector(|| "setimage-confirm".into()).child(
                    Button::new("setimage-confirm")
                        .label(if applying {
                            SharedString::from("Setting…")
                        } else {
                            label
                        })
                        .primary()
                        .disabled(!self.can_submit(cx))
                        .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                ),
            )
    }
}

impl Render for SetImageDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        v_flex()
            .id("setimage-dialog")
            .debug_selector(|| "setimage-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(560.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.submit(cx)))
            .child(DialogHeader::new().child(DialogTitle::new().child(self.title())))
            .child(self.body(cx))
            .child(self.footer(cx))
    }
}
