//! Drawing the debug dialog: what it does, the fields, the target choice, the progress.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Selectable as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dialog::{DebugDialog, DebugStage};

/// What the dialog says about the container it adds: it stays in the pod for good.
pub const PERMANENCE_NOTE: &str = "An ephemeral container cannot be removed or edited, and its \
     name cannot be reused, until the pod is deleted. Closing the terminal ends only your \
     session; exiting the shell ends the container's main process.";

impl DebugDialog {
    fn title(&self) -> SharedString {
        format!("Debug {}", self.defaults.pod.name).into()
    }

    fn labelled(
        &self,
        label: &'static str,
        id: &'static str,
        field: AnyElement,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = cx.colors();
        v_flex()
            .gap(u(px(4.)))
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(colors.text_muted)
                    .child(label),
            )
            .child(div().debug_selector(move || id.into()).child(field))
    }

    /// The containers the debug container can share processes with: a choice for several, a
    /// line for one.
    fn target_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let targets = &self.defaults.targets;
        let field =
            if let [only] = targets.as_slice() {
                div()
                    .debug_selector(|| "debug-target".into())
                    .child(only.label())
                    .into_any_element()
            } else {
                h_flex()
                    .gap(u(px(4.)))
                    .flex_wrap()
                    .children(targets.iter().enumerate().map(|(index, container)| {
                        div()
                            .debug_selector(move || format!("debug-target-{index}"))
                            .child(
                                Button::new(("debug-target", index))
                                    .label(container.label())
                                    .small()
                                    .selected(index == self.target)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_target(index, cx)
                                    })),
                            )
                    }))
                    .into_any_element()
            };
        v_flex()
            .gap(u(px(4.)))
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(colors.text_muted)
                    .child("Share the processes of"),
            )
            .child(field)
            .into_any_element()
    }

    fn editing_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        v_flex()
            .gap(u(px(12.)))
            .child(
                div().text_color(colors.text_muted).child(
                    "Adds a tool-rich container to the running pod and opens a terminal in it.",
                ),
            )
            .child(
                h_flex()
                    .debug_selector(|| "debug-warning".into())
                    .gap(u(px(8.)))
                    .items_start()
                    .text_size(u(px(12.)))
                    .text_color(colors.warning)
                    .child(Icon::new(IconName::TriangleAlert).size(u(px(14.))))
                    .child(div().flex_1().min_w_0().child(PERMANENCE_NOTE)),
            )
            .child(self.labelled(
                "Image",
                "debug-image",
                Input::new(&self.image).into_any_element(),
                cx,
            ))
            .child(self.target_row(cx))
            .child(self.labelled(
                "Command",
                "debug-command",
                Input::new(&self.command).into_any_element(),
                cx,
            ))
            .child(self.labelled(
                "Name (optional)",
                "debug-name",
                Input::new(&self.name).into_any_element(),
                cx,
            ))
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "debug-error".into())
                    .text_color(colors.error)
                    .child(error)
            }))
            .into_any_element()
    }

    fn starting_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        h_flex()
            .debug_selector(|| "debug-starting".into())
            .gap(u(px(8.)))
            .items_center()
            .child(
                Spinner::new()
                    .icon(Icon::new(IconName::LoaderCircle))
                    .color(colors.accent),
            )
            .child("Starting the debug container… pulling the image can take a while.")
            .into_any_element()
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        match self.stage {
            DebugStage::Editing => DialogFooter::new()
                .child(
                    Button::new("debug-cancel")
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                )
                .child(
                    div().debug_selector(|| "debug-confirm".into()).child(
                        Button::new("debug-confirm")
                            .label("Add debug container")
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
                ),
            DebugStage::Starting => DialogFooter::new(),
        }
    }
}

impl Render for DebugDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = match self.stage {
            DebugStage::Editing => self.editing_body(cx),
            DebugStage::Starting => self.starting_body(cx),
        };
        v_flex()
            .id("debug-dialog")
            .debug_selector(|| "debug-dialog".into())
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
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.submit(cx)))
            .child(DialogHeader::new().child(DialogTitle::new().child(self.title())))
            .child(body)
            .child(self.footer(cx))
    }
}
