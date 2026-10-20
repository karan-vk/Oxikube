//! Drawing the start dialog.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::checkbox::Checkbox;
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::{Input, InputState};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, Tokens, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::view::{Stage, StartDialog};

impl Render for StartDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let starting = self.stage == Stage::Starting;
        let input = self.input(cx);
        let errors = self.form.errors(&input);
        let note = |id: &'static str, text: Option<String>, error: bool| {
            text.map(|text| {
                div()
                    .debug_selector(move || id.into())
                    .text_size(u(px(12.)))
                    .text_color(if error { colors.error } else { colors.warning })
                    .child(text)
            })
        };
        let body = v_flex()
            .gap(u(px(10.)))
            .children(self.suggestion_list(&tokens, cx))
            .child(self.port_chips(&tokens, cx))
            .child(field(&tokens, "Remote port", "pf-remote", &self.remote))
            .children(note("pf-remote-problem", errors.remote_port.clone(), true))
            .child(self.local_field(&tokens, cx))
            .children(note("pf-local-problem", errors.local_port.clone(), true))
            .children(note(
                "pf-local-warning",
                self.form.local_warning(&input),
                false,
            ))
            .child(field(&tokens, "Bind address", "pf-bind", &self.bind))
            .children(note("pf-bind-problem", errors.bind, true))
            .children(note(
                "pf-bind-warning",
                self.form.bind_warning(&input),
                false,
            ))
            .child(field(&tokens, "Name", "pf-name", &self.name))
            .child(
                div().debug_selector(|| "pf-favourite".into()).child(
                    Checkbox::new("pf-favourite-box")
                        .label("Keep as a favourite")
                        .checked(self.favourite)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.set_favourite(*checked, cx)
                        })),
                ),
            )
            .children(self.problem_view(&tokens, cx))
            .children(starting.then(|| {
                h_flex()
                    .debug_selector(|| "pf-starting".into())
                    .gap(u(px(8.)))
                    .items_center()
                    .child(
                        Spinner::new()
                            .icon(Icon::new(IconName::LoaderCircle))
                            .color(colors.accent),
                    )
                    .child("Starting…")
            }));
        let footer = DialogFooter::new()
            .child(
                Button::new("pf-cancel")
                    .label("Cancel")
                    .disabled(starting)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
            .child(
                div().debug_selector(|| "pf-submit".into()).child(
                    Button::new("pf-submit-button")
                        .label("Forward")
                        .primary()
                        .disabled(!self.can_submit(cx))
                        .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                ),
            );
        v_flex()
            .id("portforward-start-dialog")
            .debug_selector(|| "portforward-start-dialog".into())
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
            .child(footer)
    }
}

fn field(
    tokens: &Tokens,
    label: &'static str,
    id: &'static str,
    input: &Entity<InputState>,
) -> AnyElement {
    v_flex()
        .gap(u(px(4.)))
        .child(
            div()
                .text_size(u(px(12.)))
                .text_color(tokens.colors.text_muted)
                .child(label),
        )
        .child(
            div()
                .debug_selector(move || id.into())
                .child(Input::new(input)),
        )
        .into_any_element()
}

impl StartDialog {
    /// The target's ports as chips; picking one fills the remote port.
    fn port_chips(&self, tokens: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let colors = tokens.colors;
        if self.form.ports.is_empty() {
            return div()
                .debug_selector(|| "pf-ports-none".into())
                .text_size(u(px(12.)))
                .text_color(colors.text_muted)
                .child(if self.loading_ports {
                    "Reading the target's ports…"
                } else {
                    "Type the port to forward."
                })
                .into_any_element();
        }
        h_flex()
            .debug_selector(|| "pf-ports".into())
            .flex_wrap()
            .gap(u(px(6.)))
            .children(self.form.ports.iter().enumerate().map(|(i, port)| {
                let choice = port.clone();
                let label: SharedString = port.label().into();
                div().debug_selector(move || format!("pf-port-{i}")).child(
                    Button::new(("pf-port", i))
                        .xsmall()
                        .outline()
                        .label(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_port(&choice, window, cx)
                        })),
                )
            }))
            .into_any_element()
    }

    fn local_field(&self, tokens: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap(u(px(4.)))
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(tokens.colors.text_muted)
                    .child("Local port"),
            )
            .child(
                h_flex()
                    .gap(u(px(10.)))
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "pf-local".into())
                            .flex_1()
                            .child(Input::new(&self.local)),
                    )
                    .child(
                        div().debug_selector(|| "pf-auto".into()).child(
                            Checkbox::new("pf-auto-box")
                                .label("Auto")
                                .checked(self.local_auto)
                                .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                    this.set_auto(*checked, window, cx)
                                })),
                        ),
                    ),
            )
            .into_any_element()
    }

    /// The failed start under the fields: the message, the suggestion and the fix button.
    fn problem_view(&self, tokens: &Tokens, cx: &mut Context<Self>) -> Option<AnyElement> {
        let problem = self.problem.as_ref()?;
        let colors = tokens.colors;
        let fix = problem.fix;
        Some(
            v_flex()
                .debug_selector(|| "pf-problem".into())
                .gap(u(px(4.)))
                .p(u(px(8.)))
                .rounded(u(tokens.radius.md))
                .bg(colors.element)
                .child(
                    div()
                        .debug_selector(|| "pf-problem-message".into())
                        .text_color(colors.error)
                        .child(problem.message.clone()),
                )
                .children(problem.suggestion.clone().map(|text| {
                    div()
                        .debug_selector(|| "pf-problem-suggestion".into())
                        .text_size(u(px(12.)))
                        .text_color(colors.text_muted)
                        .child(text)
                }))
                .when_some(fix, |column, fix| {
                    column.child(div().debug_selector(|| "pf-problem-fix".into()).child(
                        Button::new("pf-fix").xsmall().label(fix.label()).on_click(
                            cx.listener(move |this, _, window, cx| this.apply_fix(fix, window, cx)),
                        ),
                    ))
                })
                .into_any_element(),
        )
    }
}

impl StartDialog {
    /// "Suggested from annotations": one row per k9s annotation entry; a click fills the fields.
    /// `None` without suggestions (a plain pod, or the setting off).
    fn suggestion_list(&self, tokens: &Tokens, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.suggestions.is_empty() {
            return None;
        }
        let colors = tokens.colors;
        let rows = self.suggestions.iter().enumerate().map(|(i, suggestion)| {
            let mut label = suggestion.label();
            if suggestion.forward.auto {
                label.push_str(" (auto)");
            }
            let button = Button::new(("pf-suggestion", i))
                .xsmall()
                .label(SharedString::from(label))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.use_suggestion(i, window, cx)),
                );
            let button = if self.applied == Some(i) {
                button.primary()
            } else {
                button.outline()
            };
            h_flex()
                .gap(u(px(8.)))
                .items_center()
                .child(
                    div()
                        .debug_selector(move || format!("pf-suggestion-{i}"))
                        .child(button),
                )
                .children(suggestion.conflict.clone().map(|text| {
                    div()
                        .debug_selector(move || format!("pf-suggestion-conflict-{i}"))
                        .text_size(u(px(12.)))
                        .text_color(colors.warning)
                        .child(text)
                }))
        });
        Some(
            v_flex()
                .debug_selector(|| "pf-suggestions".into())
                .gap(u(px(6.)))
                .child(
                    div()
                        .text_size(u(px(12.)))
                        .text_color(colors.text_muted)
                        .child("Suggested from annotations"),
                )
                .children(rows)
                .into_any_element(),
        )
    }
}
