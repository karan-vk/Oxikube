//! Drawing the "Create secret" dialog.

use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_app::NewSecretField;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::{Input, Textarea};
use oxikube_ui::layout::{Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dialog::{CreateSecretDialog, SecretKind};

/// What the dialog says about where the values go.
pub const NOTE: &str = "The Secret opens in an editor with its values as text. Review it there \
    and apply: it is base64-encoded then, and the preview hides the values.";

impl CreateSecretDialog {
    /// `field` under a small muted `label`, with the problem of `at` in red below.
    fn labelled(
        &self,
        label: &'static str,
        id: &'static str,
        at: NewSecretField,
        field: AnyElement,
        cx: &mut Context<Self>,
    ) -> Div {
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
            .children(self.problem(at).map(|message| {
                div()
                    .debug_selector(move || format!("{id}-problem"))
                    .text_size(u(px(12.)))
                    .text_color(colors.error)
                    .child(message.to_owned())
            }))
    }

    fn kind_row(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap(u(px(4.)))
            .children(SecretKind::ALL.into_iter().map(|kind| {
                div()
                    .debug_selector(move || format!("secret-kind-{}", kind.label()))
                    .child(
                        Button::new(("secret-kind", kind as usize))
                            .label(kind.label())
                            .small()
                            .selected(kind == self.kind)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.select_kind(kind, cx)),
                            ),
                    )
            }))
            .into_any_element()
    }

    fn opaque_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        v_flex()
            .gap(u(px(8.)))
            .children(self.rows.iter().enumerate().map(|(index, row)| {
                let key_problem = self.problem(NewSecretField::Key(index));
                let value_problem = self.problem(NewSecretField::Value(index));
                v_flex()
                    .gap(u(px(2.)))
                    .child(
                        h_flex()
                            .gap(u(px(8.)))
                            .items_start()
                            .child(
                                div()
                                    .w(u(px(180.)))
                                    .flex_none()
                                    .debug_selector(move || format!("secret-key-{index}"))
                                    .child(Input::new(&row.key)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .debug_selector(move || format!("secret-value-{index}"))
                                    .child(Textarea::new(&row.value)),
                            )
                            .child(
                                Button::new(("secret-remove", index))
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::X))
                                    .tooltip("Remove this key")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_row(index, cx)
                                    })),
                            ),
                    )
                    .children(key_problem.map(|message| problem_line(message, colors.error)))
                    .children(value_problem.map(|message| problem_line(message, colors.error)))
            }))
            .child(
                div().child(
                    Button::new("secret-add-row")
                        .label("Add key")
                        .ghost()
                        .small()
                        .icon(Icon::new(IconName::Plus))
                        .on_click(cx.listener(|this, _, window, cx| this.add_row(window, cx))),
                ),
            )
            .children(
                self.problem(NewSecretField::Secret)
                    .map(|message| problem_line(message, colors.error)),
            )
            .into_any_element()
    }

    fn docker_body(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap(u(px(8.)))
            .child(self.labelled(
                "Registry",
                "secret-server",
                NewSecretField::Server,
                Input::new(&self.server).into_any_element(),
                cx,
            ))
            .child(self.labelled(
                "Username",
                "secret-username",
                NewSecretField::Username,
                Input::new(&self.username).into_any_element(),
                cx,
            ))
            .child(self.labelled(
                "Password",
                "secret-password",
                NewSecretField::Password,
                Input::new(&self.password).into_any_element(),
                cx,
            ))
            .child(self.labelled(
                "Email (optional)",
                "secret-email",
                NewSecretField::Email,
                Input::new(&self.email).into_any_element(),
                cx,
            ))
            .into_any_element()
    }

    fn tls_body(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap(u(px(8.)))
            .child(self.labelled(
                "Certificate (PEM)",
                "secret-certificate",
                NewSecretField::Certificate,
                Textarea::new(&self.certificate).into_any_element(),
                cx,
            ))
            .child(self.labelled(
                "Private key (PEM)",
                "secret-private-key",
                NewSecretField::PrivateKey,
                Textarea::new(&self.private_key).into_any_element(),
                cx,
            ))
            .into_any_element()
    }
}

fn problem_line(message: &str, color: gpui::Hsla) -> Div {
    div()
        .text_size(u(px(12.)))
        .text_color(color)
        .child(message.to_owned())
}

impl Render for CreateSecretDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = match self.kind {
            SecretKind::Opaque => self.opaque_body(cx),
            SecretKind::Docker => self.docker_body(cx),
            SecretKind::Tls => self.tls_body(cx),
        };
        let name = self.labelled(
            "Name",
            "secret-name",
            NewSecretField::Name,
            Input::new(&self.name).into_any_element(),
            cx,
        );
        let namespace = self.labelled(
            "Namespace",
            "secret-namespace",
            NewSecretField::Namespace,
            Input::new(&self.namespace).into_any_element(),
            cx,
        );
        v_flex()
            .id("create-secret-dialog")
            .debug_selector(|| "create-secret-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(600.)))
            .max_h(u(px(640.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, window, cx| this.cancel(window, cx)))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| {
                if this.kind() != SecretKind::Opaque {
                    this.submit(window, cx);
                }
            }))
            .child(DialogHeader::new().child(DialogTitle::new().child("Create secret")))
            .child(self.kind_row(cx))
            .child(
                h_flex()
                    .gap(u(px(12.)))
                    .child(name.flex_1())
                    .child(namespace.w(u(px(180.)))),
            )
            .child(
                div()
                    .id("create-secret-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(colors.text_muted)
                    .child(NOTE),
            )
            .child(
                DialogFooter::new()
                    .child(
                        Button::new("secret-cancel")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                    )
                    .child(
                        div().debug_selector(|| "secret-create".into()).child(
                            Button::new("secret-create")
                                .label("Create")
                                .primary()
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.submit(window, cx)),
                                ),
                        ),
                    ),
            )
    }
}
