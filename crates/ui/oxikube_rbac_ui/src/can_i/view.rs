//! Drawing the can-i dialog.

use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, px,
};
use oxikube_app::tools::rbac::CanIAnswer;
use oxikube_domain::access::AccessDecision;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dialog::{CanIDialog, CheckState, SubjectKind};

/// The verbs offered as buttons; any other verb is typed.
pub const QUICK_VERBS: [&str; 7] = [
    "get", "list", "watch", "create", "update", "patch", "delete",
];

impl CanIDialog {
    fn labelled(
        &self,
        label: &'static str,
        id: &'static str,
        field: AnyElement,
        cx: &mut Context<Self>,
    ) -> Div {
        v_flex()
            .gap(u(px(4.)))
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(cx.colors().text_muted)
                    .child(label),
            )
            .child(div().debug_selector(move || id.into()).child(field))
    }

    fn verb_row(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap(u(px(4.)))
            .children(QUICK_VERBS.into_iter().enumerate().map(|(index, verb)| {
                Button::new(("can-i-verb", index))
                    .label(verb)
                    .ghost()
                    .small()
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.set_verb(verb, window, cx)),
                    )
            }))
            .into_any_element()
    }

    fn subject_row(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap(u(px(4.)))
            .children(SubjectKind::ALL.into_iter().map(|kind| {
                div()
                    .debug_selector(move || format!("can-i-subject-{}", kind.label()))
                    .child(
                        Button::new(("can-i-subject", kind as usize))
                            .label(kind.label())
                            .small()
                            .selected(kind == self.subject_kind)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_subject_kind(kind, cx);
                            })),
                    )
            }))
            .into_any_element()
    }

    fn result(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let muted = |text: String| {
            div()
                .text_size(u(px(12.)))
                .text_color(colors.text_muted)
                .child(text)
        };
        let body = match &self.state {
            CheckState::Idle => muted("Ask to see the answer and the reason.".to_owned()),
            CheckState::Checking => muted("Asking the cluster...".to_owned()),
            CheckState::Failed(why) => div().text_color(colors.error).child(why.clone()),
            CheckState::Answered(answer) => {
                answered(answer, colors.success, colors.error, colors.warning)
            }
        };
        div()
            .debug_selector(|| "can-i-result".into())
            .min_h(u(px(96.)))
            .child(body)
            .into_any_element()
    }
}

fn answered(answer: &CanIAnswer, ok: gpui::Hsla, no: gpui::Hsla, maybe: gpui::Hsla) -> Div {
    let colour = match answer.decision {
        AccessDecision::Allowed => ok,
        AccessDecision::Denied { .. } => no,
        AccessDecision::Unknown { .. } => maybe,
    };
    v_flex()
        .gap(u(px(4.)))
        .child(
            div()
                .debug_selector(|| "can-i-outcome".into())
                .text_size(u(px(18.)))
                .text_color(colour)
                .child(answer.outcome()),
        )
        .child(div().child(answer.question()))
        .child(
            div()
                .debug_selector(|| "can-i-reason".into())
                .child(format!("Reason: {}", answer.reason())),
        )
        .child(
            div()
                .debug_selector(|| "can-i-method".into())
                .text_size(u(px(12.)))
                .child(format!("Answered by: {}", answer.method.label())),
        )
        .child(
            div()
                .text_size(u(px(12.)))
                .child(format!("kubectl: {}", answer.kubectl())),
        )
}

impl Render for CanIDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let subject_field = self.subject_kind != SubjectKind::Myself;
        v_flex()
            .id("can-i-dialog")
            .debug_selector(|| "can-i-dialog".into())
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
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.ask(window, cx)))
            .child(DialogHeader::new().child(DialogTitle::new().child("Can I?")))
            .child(self.labelled(
                "Verb",
                "can-i-verb",
                Input::new(&self.verb).into_any_element(),
                cx,
            ))
            .child(self.verb_row(cx))
            .child(self.labelled(
                "Resource",
                "can-i-resource",
                Input::new(&self.resource).into_any_element(),
                cx,
            ))
            .child(
                h_flex()
                    .gap(u(px(12.)))
                    .child(
                        self.labelled(
                            "Namespace",
                            "can-i-namespace",
                            Input::new(&self.namespace).into_any_element(),
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(
                        self.labelled(
                            "Name (optional)",
                            "can-i-name",
                            Input::new(&self.name).into_any_element(),
                            cx,
                        )
                        .flex_1(),
                    ),
            )
            .child(self.subject_row(cx))
            .children(subject_field.then(|| {
                self.labelled(
                    "Subject (impersonated, not saved)",
                    "can-i-subject",
                    Input::new(&self.subject).into_any_element(),
                    cx,
                )
            }))
            .child(self.result(cx))
            .child(
                DialogFooter::new()
                    .child(
                        Button::new("can-i-copy")
                            .label("Copy result")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.copy_result(cx);
                            })),
                    )
                    .child(
                        Button::new("can-i-close")
                            .label("Close")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        div().debug_selector(|| "can-i-ask".into()).child(
                            Button::new("can-i-ask")
                                .label("Ask")
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| this.ask(window, cx))),
                        ),
                    ),
            )
    }
}
