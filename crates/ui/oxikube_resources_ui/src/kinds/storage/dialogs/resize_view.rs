//! Drawing the resize dialog.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::resize::{Loaded, ResizeDialog, ResizeStage};
use oxikube_domain::view::storage::Expansion;

impl ResizeDialog {
    fn what(&self) -> String {
        let target = self.target();
        let object = match target.namespace() {
            Some(ns) => format!("{ns}/{}", target.name),
            None => target.name.to_string(),
        };
        match &self.loaded {
            Loaded::Ready(context) => format!("{object} on {}", context.context),
            _ => object,
        }
    }

    fn fact(
        &self,
        id: &'static str,
        label: &'static str,
        value: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.colors();
        h_flex()
            .debug_selector(move || id.to_owned())
            .gap(u(px(8.)))
            .child(
                div()
                    .w(u(px(110.)))
                    .text_color(colors.text_muted)
                    .child(label),
            )
            .child(div().child(value))
            .into_any_element()
    }

    fn asking_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let Loaded::Ready(context) = &self.loaded else {
            return div().into_any_element();
        };
        let class = match (&context.claim.storage_class, &context.expansion) {
            (Some(class), Expansion::Allowed) => format!("{class} (volume expansion allowed)"),
            (Some(class), Expansion::Disallowed { .. }) => {
                format!("{class} (volume expansion not allowed)")
            }
            (Some(class), _) => format!("{class} (could not be read)"),
            (None, _) => "none".to_owned(),
        };
        let size = context.claim.size_text();
        let check = self.check(cx);
        let blocked = self.blocked();
        // What is wrong, if anything: the class first (nothing typed can fix it), then the text.
        let problem = blocked.clone().or_else(|| {
            let typed = self.typed(cx);
            match check {
                Some(Err(block)) if !typed.is_empty() => Some(block),
                _ => None,
            }
        });
        v_flex()
            .gap(u(px(10.)))
            .child(
                div()
                    .debug_selector(|| "resize-what".into())
                    .text_color(colors.text_muted)
                    .child(self.what()),
            )
            .child(self.fact("resize-current", "Current size", size, cx))
            .child(self.fact("resize-class", "Storage class", class, cx))
            .child(
                div()
                    .debug_selector(|| "resize-input".into())
                    .child(Input::new(&self.size).disabled(blocked.is_some())),
            )
            .children(problem.map(|problem| {
                h_flex()
                    .debug_selector(|| "resize-problem".into())
                    .gap(u(px(8.)))
                    .items_center()
                    .text_color(colors.warning)
                    .child(Icon::new(IconName::TriangleAlert).size(u(px(14.))))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .child(problem.to_string()),
                    )
            }))
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "resize-error".into())
                    .text_color(colors.error)
                    .child(error)
            }))
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(colors.text_muted)
                    .child("A volume can only grow: it cannot be shrunk again."),
            )
            .into_any_element()
    }

    fn running_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        h_flex()
            .debug_selector(|| "resize-running".into())
            .gap(u(px(8.)))
            .items_center()
            .child(
                Spinner::new()
                    .icon(Icon::new(IconName::LoaderCircle))
                    .color(colors.accent),
            )
            .child("Resizing…")
            .into_any_element()
    }

    fn done_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let Some(report) = self.report() else {
            return div().into_any_element();
        };
        v_flex()
            .gap(u(px(8.)))
            .child(
                div()
                    .debug_selector(|| "resize-summary".into())
                    .text_color(colors.success)
                    .child(report.message.clone()),
            )
            .children(report.conditions.iter().enumerate().map(|(i, condition)| {
                let text = match &condition.message {
                    Some(message) => format!("{}: {message}", condition.kind),
                    None => condition.kind.to_string(),
                };
                div()
                    .debug_selector(move || format!("resize-condition-{i}"))
                    .text_color(colors.text_muted)
                    .whitespace_normal()
                    .child(text)
            }))
            .into_any_element()
    }

    fn footer(&self, cx: &mut Context<Self>) -> DialogFooter {
        if matches!(self.loaded, Loaded::Failed(_)) {
            return DialogFooter::new().child(
                Button::new("resize-close")
                    .label("Close")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            );
        }
        let enabled = self.can_confirm(cx);
        match self.stage {
            ResizeStage::Asking => DialogFooter::new()
                .child(
                    Button::new("resize-cancel")
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                )
                .child(
                    div().debug_selector(|| "resize-confirm".into()).child(
                        Button::new("resize-confirm")
                            .label("Resize")
                            .primary()
                            .disabled(!enabled)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                    ),
                ),
            ResizeStage::Running => DialogFooter::new(),
            ResizeStage::Done => DialogFooter::new().child(
                div().debug_selector(|| "resize-close".into()).child(
                    Button::new("resize-close")
                        .label("Close")
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                ),
            ),
        }
    }
}

impl Render for ResizeDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = match (&self.loaded, self.stage) {
            (Loaded::Loading, _) => h_flex()
                .debug_selector(|| "resize-loading".into())
                .gap(u(px(8.)))
                .items_center()
                .child(
                    Spinner::new()
                        .icon(Icon::new(IconName::LoaderCircle))
                        .color(colors.accent),
                )
                .child("Reading the claim…")
                .into_any_element(),
            (Loaded::Failed(reason), _) => div()
                .debug_selector(|| "resize-load-error".into())
                .text_color(colors.error)
                .child(reason.clone())
                .into_any_element(),
            (Loaded::Ready(_), ResizeStage::Asking) => self.asking_body(cx),
            (Loaded::Ready(_), ResizeStage::Running) => self.running_body(cx),
            (Loaded::Ready(_), ResizeStage::Done) => self.done_body(cx),
        };
        let footer = self.footer(cx);
        v_flex()
            .id("resize-dialog")
            .debug_selector(|| "resize-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(520.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &oxikube_ui::dialog::Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(
                |this, _: &oxikube_ui::dialog::Confirm, _, cx| match this.stage {
                    ResizeStage::Asking => this.confirm(cx),
                    ResizeStage::Done => this.cancel(cx),
                    ResizeStage::Running => {}
                },
            ))
            .child(DialogHeader::new().child(DialogTitle::new().child("Resize volume claim")))
            .child(body)
            .child(footer)
    }
}
