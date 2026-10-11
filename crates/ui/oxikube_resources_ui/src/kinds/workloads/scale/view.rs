//! Drawing the scale dialog: what is scaled, the current count, the field, the warnings.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dialog::{ScaleDialog, ScaleStage};

/// What the dialog says when the count is zero.
pub(super) const ZERO_NOTE: &str =
    "Scaling to 0 stops every pod of this workload. It serves nothing until it is scaled up again.";

fn replicas_word(n: u32) -> &'static str {
    if n == 1 { "replica" } else { "replicas" }
}

impl ScaleDialog {
    fn title(&self) -> SharedString {
        format!("Scale {}", self.target.name).into()
    }

    /// `Deployment shop/web`.
    fn subtitle(&self) -> SharedString {
        let kind = &self.target.gvk.kind;
        match self.target.namespace() {
            Some(ns) => format!("{kind} {ns}/{}", self.target.name),
            None => format!("{kind} {}", self.target.name),
        }
        .into()
    }

    /// The line about the count now: running, and wanted when that differs.
    fn current_line(&self) -> Option<String> {
        let plan = self.plan.as_ref()?;
        let running = format!("{} {} running", plan.current, replicas_word(plan.current));
        Some(if plan.current == plan.desired {
            running
        } else {
            format!("{running}, {} wanted", plan.desired)
        })
    }

    fn notes(&self, count: Option<u32>, cx: &mut Context<Self>) -> Vec<AnyElement> {
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
        let mut notes = Vec::new();
        if count == Some(0) {
            notes.push(note("scale-zero", colors.warning, ZERO_NOTE.to_owned()));
        }
        if let Some(name) = self.plan.as_ref().and_then(|plan| plan.autoscaler.clone()) {
            notes.push(note(
                "scale-autoscaler",
                colors.text_muted,
                format!(
                    "HorizontalPodAutoscaler {name} targets this workload; it may set the \
                     count back."
                ),
            ));
        }
        notes
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let parsed = self.parsed(cx);
        let loading = self.stage == ScaleStage::Loading;
        let field_error = match (&parsed, self.stage) {
            (Err(error), ScaleStage::Editing) if !self.text(cx).trim().is_empty() => {
                Some(error.message().to_owned())
            }
            _ => None,
        };
        let stepper = |id: &'static str, icon: IconName, delta: i64, cx: &mut Context<Self>| {
            div().debug_selector(move || id.into()).child(
                Button::new(id)
                    .small()
                    .icon(Icon::new(icon).size(u(px(14.))))
                    .disabled(self.stage != ScaleStage::Editing)
                    .on_click(cx.listener(move |this, _, window, cx| this.step(delta, window, cx))),
            )
        };
        let current = if loading {
            h_flex()
                .debug_selector(|| "scale-loading".into())
                .gap(u(px(8.)))
                .items_center()
                .text_color(colors.text_muted)
                .child(
                    Spinner::new()
                        .icon(Icon::new(IconName::LoaderCircle))
                        .color(colors.accent),
                )
                .child("Reading the current replicas…")
                .into_any_element()
        } else {
            div()
                .debug_selector(|| "scale-current".into())
                .text_color(colors.text_muted)
                .child(
                    self.current_line()
                        .unwrap_or_else(|| "The current count could not be read.".to_owned()),
                )
                .into_any_element()
        };
        v_flex()
            .gap(u(px(12.)))
            .child(
                div()
                    .debug_selector(|| "scale-subtitle".into())
                    .text_color(colors.text_muted)
                    .child(self.subtitle()),
            )
            .child(current)
            .child(
                v_flex()
                    .gap(u(px(4.)))
                    .child(
                        div()
                            .text_size(u(px(12.)))
                            .text_color(colors.text_muted)
                            .child("Replicas"),
                    )
                    .child(
                        h_flex()
                            .gap(u(px(4.)))
                            .items_center()
                            .child(stepper("scale-minus", IconName::Minus, -1, cx))
                            .child(
                                div()
                                    .debug_selector(|| "scale-replicas".into())
                                    .flex_1()
                                    .child(Input::new(&self.replicas).disabled(loading)),
                            )
                            .child(stepper("scale-plus", IconName::Plus, 1, cx)),
                    ),
            )
            .children(field_error.map(|message| {
                div()
                    .debug_selector(|| "scale-field-error".into())
                    .text_color(colors.error)
                    .child(message)
            }))
            .children(self.notes(parsed.as_ref().ok().copied(), cx))
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "scale-error".into())
                    .text_color(colors.error)
                    .child(error)
            }))
            .into_any_element()
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let label: SharedString = match self.parsed(cx) {
            Ok(n) => format!("Scale to {n}").into(),
            Err(_) => "Scale".into(),
        };
        let scaling = self.stage == ScaleStage::Scaling;
        DialogFooter::new()
            .child(
                Button::new("scale-cancel")
                    .label("Cancel")
                    .disabled(scaling)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
            .child(
                div().debug_selector(|| "scale-confirm".into()).child(
                    Button::new("scale-confirm")
                        .label(if scaling {
                            SharedString::from("Scaling…")
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

impl Render for ScaleDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        v_flex()
            .id("scale-dialog")
            .debug_selector(|| "scale-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(440.)))
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
