//! Drawing the confirmation dialog: title and summary, the context panel (cluster, initiator,
//! risk), the dry-run banner, the object list and the typed-name field.

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _, px, uniform_list,
};
use oxikube_domain::audit::Initiator;
use oxikube_domain::safety::Risk;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{
    Cancel, Confirm, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Colors, u};

use super::{ConfirmDialog, PreviewNote};
use crate::cluster::badge_colour;
use crate::modal::dialog::{BUTTON_KEY_CONTEXT, DIALOG_KEY_CONTEXT};

/// Height of one object row, unscaled pixels (`uniform_list` rows share one height).
const ROW_HEIGHT: f32 = 22.0;

/// How many object rows show before the list scrolls.
const VISIBLE_ROWS: usize = 8;

fn initiator_label(initiator: Initiator) -> &'static str {
    match initiator {
        Initiator::Ui => "You",
        Initiator::Command => "A command",
        Initiator::Agent => "An agent",
        Initiator::Plugin => "A plugin",
    }
}

fn risk_label(risk: Risk) -> &'static str {
    match risk {
        Risk::Low => "Low risk",
        Risk::Medium => "Medium risk",
        Risk::High => "High risk",
        Risk::Irreversible => "Irreversible",
    }
}

fn risk_colour(risk: Risk, colors: &Colors) -> Hsla {
    match risk {
        Risk::Low => colors.text_muted,
        Risk::Medium => colors.warning,
        Risk::High | Risk::Irreversible => colors.error,
    }
}

impl ConfirmDialog {
    fn context_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let prompt = &self.prompt;
        let by_other = prompt.initiator != Initiator::Ui;
        let who = if by_other {
            format!("{} is asking", initiator_label(prompt.initiator))
        } else {
            "Asked by you".to_owned()
        };
        v_flex()
            .debug_selector(|| "confirm-context".to_owned())
            .gap(u(tokens.spacing.xs))
            .p(u(tokens.spacing.md))
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.md))
            .children(prompt.cluster.as_ref().map(|(name, colour)| {
                h_flex()
                    .gap(u(tokens.spacing.sm))
                    .items_center()
                    .child(div().text_color(colors.text_muted).child("Cluster"))
                    .children(colour.map(|colour| {
                        div()
                            .debug_selector(|| "confirm-cluster-dot".to_owned())
                            .flex_none()
                            .size(u(px(8.)))
                            .rounded_full()
                            .bg(badge_colour(colour, &tokens))
                    }))
                    .child(
                        div()
                            .debug_selector(|| "confirm-cluster".to_owned())
                            .child(name.clone()),
                    )
            }))
            .child(
                h_flex()
                    .gap(u(tokens.spacing.sm))
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "confirm-initiator".to_owned())
                            .when(by_other, |this| this.text_color(colors.warning))
                            .child(who),
                    )
                    .children(prompt.risk.map(|risk| {
                        div()
                            .debug_selector(|| "confirm-risk".to_owned())
                            .text_color(risk_colour(risk, &colors))
                            .child(risk_label(risk))
                    })),
            )
            .into_any_element()
    }

    fn preview_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let (text, colour): (SharedString, Hsla) = match self.prompt.preview.as_ref()? {
            PreviewNote::Rehearsed(message) => (message.clone(), colors.success),
            PreviewNote::Unavailable(reason) => (
                format!("No preview available: {reason}").into(),
                colors.warning,
            ),
        };
        Some(
            div()
                .debug_selector(|| "confirm-preview".to_owned())
                .text_color(colour)
                .child(text)
                .into_any_element(),
        )
    }

    /// The bulk object list: a count line and a `uniform_list` that builds only the rows on
    /// screen, however many objects there are.
    fn items_list(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let count = self.prompt.items.len();
        if count == 0 {
            return None;
        }
        let colors = cx.colors();
        let height = u(px(ROW_HEIGHT * count.min(VISIBLE_ROWS) as f32));
        let show_risk = self.prompt.items_differ_in_risk();
        Some(
            v_flex()
                .gap(u(px(4.)))
                .child(
                    div()
                        .debug_selector(|| "confirm-count".to_owned())
                        .text_color(colors.text_muted)
                        .child(format!("{count} objects")),
                )
                .child(
                    div()
                        .debug_selector(|| "confirm-items".to_owned())
                        .h(height)
                        .w_full()
                        .child(
                            uniform_list(
                                "confirm-items-list",
                                count,
                                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                    let colors = cx.colors();
                                    range
                                        .filter_map(|ix| {
                                            let item = this.prompt.items.get(ix)?;
                                            Some(
                                                h_flex()
                                                    .id(("confirm-item", ix))
                                                    .debug_selector(move || {
                                                        format!("confirm-item-{ix}")
                                                    })
                                                    .h(u(px(ROW_HEIGHT)))
                                                    .gap(u(px(8.)))
                                                    .items_center()
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .truncate()
                                                            .child(item.label.clone()),
                                                    )
                                                    .when(show_risk, |row| {
                                                        row.child(
                                                            div()
                                                                .text_color(risk_colour(
                                                                    item.risk, &colors,
                                                                ))
                                                                .child(risk_label(item.risk)),
                                                        )
                                                    })
                                                    .into_any_element(),
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .size_full(),
                        ),
                )
                .into_any_element(),
        )
    }
}

impl Render for ConfirmDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let prompt = &self.prompt;
        let mut confirm = Button::new("dialog-confirm").label(prompt.confirm_label.clone());
        confirm = if prompt.destructive {
            confirm.danger()
        } else {
            confirm.primary()
        };
        let confirm = confirm
            .disabled(!self.can_confirm(cx))
            .on_click(cx.listener(|this, _, window, cx| this.press_confirm(window, cx)));
        let cancel = Button::new("dialog-cancel")
            .label("Cancel")
            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)));

        v_flex()
            .id("confirm-dialog")
            .debug_selector(|| "dialog-modal".to_owned())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .tab_group()
            .w(u(px(460.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.press_confirm(window, cx)))
            .child(
                DialogHeader::new()
                    .child(DialogTitle::new().child(prompt.title.clone()))
                    .child(DialogDescription::new().child(prompt.summary.clone())),
            )
            .child(self.context_panel(cx))
            .children(self.preview_banner(cx))
            .children(self.items_list(cx))
            .children(self.typed.as_ref().map(|typed| {
                v_flex()
                    .gap(u(tokens.spacing.sm))
                    .child(
                        div()
                            .debug_selector(|| "dialog-type-prompt".to_owned())
                            .child(format!("Type {} to confirm", typed.expected)),
                    )
                    .child(
                        div()
                            .debug_selector(|| "dialog-type-input".to_owned())
                            .child(Input::new(&typed.input)),
                    )
            }))
            .child(
                DialogFooter::new()
                    .child(
                        div()
                            .key_context(BUTTON_KEY_CONTEXT)
                            .debug_selector(|| "dialog-cancel".to_owned())
                            .child(cancel),
                    )
                    .child(
                        div()
                            .key_context(BUTTON_KEY_CONTEXT)
                            .debug_selector(|| "dialog-confirm".to_owned())
                            .child(confirm),
                    ),
            )
    }
}
