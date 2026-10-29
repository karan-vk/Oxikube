//! The footer: the counts, the problem that stopped a run, the confirmation of Apply, and the
//! actions of the stage.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, Styled as _, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use crate::apply::model::{Activity, Stage};
use crate::apply::view::ApplyView;

impl ApplyView {
    pub(super) fn footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let stage = self.model.stage();
        let has_rows = !self.model.rows().is_empty();
        let activity = match self.model.activity() {
            Activity::Idle => None,
            Activity::Loading => Some("Reading and ordering..."),
            Activity::DryRunning => Some("Dry run..."),
            Activity::Applying => Some("Applying..."),
        };
        let mut footer = v_flex()
            .flex_none()
            .border_t_1()
            .border_color(colors.border_variant)
            .px(u(tokens.spacing.xl))
            .py(u(tokens.spacing.md))
            .gap(u(tokens.spacing.sm));
        if let Some(problem) = self.problem.clone().filter(|_| has_rows) {
            footer = footer.child(
                div()
                    .debug_selector(|| "apply-run-problem".into())
                    .text_color(colors.error)
                    .child(SharedString::from(problem)),
            );
        }
        if self.confirming {
            return footer.child(self.confirmation(cx)).into_any_element();
        }
        let summary = if has_rows {
            self.model.summary_line()
        } else {
            String::new()
        };
        footer
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "apply-summary".into())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(colors.text_muted)
                            .child(SharedString::from(activity.map_or(summary, |text| {
                                format!("{text} {}", { self.model.summary_line() })
                            }))),
                    )
                    .when(self.read_only && has_rows, |bar| {
                        bar.child(
                            div()
                                .debug_selector(|| "apply-blocked".into())
                                .text_color(colors.warning)
                                .child(SharedString::from(
                                    self.apply_blocked().unwrap_or_default(),
                                )),
                        )
                    })
                    .when(has_rows && stage != Stage::Load, |bar| {
                        bar.child(self.view_buttons(cx))
                    })
                    .child(self.stage_buttons(cx)),
            )
            .into_any_element()
    }

    fn confirmation(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let objects = self.model.rows().len();
        let text = format!(
            "Apply {objects} {} to {}? The dry run said: {}.",
            if objects == 1 { "object" } else { "objects" },
            self.cluster_label,
            self.model.summary_line(),
        );
        h_flex()
            .debug_selector(|| "apply-confirmation".into())
            .gap(u(tokens.spacing.md))
            .items_center()
            .child(
                Icon::new(IconName::TriangleAlert)
                    .size(u(px(16.)))
                    .color(colors.warning),
            )
            .child(
                div()
                    .debug_selector(|| "apply-confirmation-text".into())
                    .flex_1()
                    .min_w_0()
                    .child(SharedString::from(text)),
            )
            .child(
                div().debug_selector(|| "apply-decline".into()).child(
                    Button::new("apply-decline-button")
                        .small()
                        .outline()
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.decline_apply(cx))),
                ),
            )
            .child(
                div().debug_selector(|| "apply-confirm".into()).child(
                    Button::new("apply-confirm-button")
                        .small()
                        .primary()
                        .label("Apply")
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_apply(cx))),
                ),
            )
            .into_any_element()
    }

    /// Errors only and Copy: for any plan on screen.
    fn view_buttons(&self, cx: &mut Context<Self>) -> AnyElement {
        let errors_only = self.model.errors_only();
        h_flex()
            .gap(u(cx.tokens().spacing.sm))
            .child(
                div().debug_selector(|| "apply-errors-only".into()).child(
                    Button::new("apply-errors-only-button")
                        .small()
                        .outline()
                        .selected(errors_only)
                        .icon(Icon::new(IconName::Funnel).size(u(px(14.))))
                        .label("Errors only")
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_errors_only(cx))),
                ),
            )
            .child(
                div().debug_selector(|| "apply-copy".into()).child(
                    Button::new("apply-copy-button")
                        .small()
                        .outline()
                        .icon(Icon::new(IconName::Copy).size(u(px(14.))))
                        .label("Copy as text")
                        .on_click(cx.listener(|this, _, _, cx| this.copy_results(cx))),
                ),
            )
            .into_any_element()
    }

    /// Apply, Cancel, Re-run and Re-run failed only.
    fn stage_buttons(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.model.busy();
        let blocked = self.apply_blocked();
        let failures = self.model.failures();
        let can_rerun = self.source.is_some() && !busy;
        let mut buttons = h_flex().gap(u(cx.tokens().spacing.sm));
        if busy {
            return buttons
                .child(
                    div().debug_selector(|| "apply-cancel".into()).child(
                        Button::new("apply-cancel-button")
                            .small()
                            .outline()
                            .icon(Icon::new(IconName::X).size(u(px(14.))))
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_run(cx))),
                    ),
                )
                .into_any_element();
        }
        buttons = buttons
            .child(
                div().debug_selector(|| "apply-rerun".into()).child(
                    Button::new("apply-rerun-button")
                        .small()
                        .outline()
                        .icon(Icon::new(IconName::RotateCcw).size(u(px(14.))))
                        .label("Re-run")
                        .tooltip("Read the files again and rehearse the plan")
                        .disabled(!can_rerun)
                        .on_click(cx.listener(|this, _, _, cx| this.rerun(cx))),
                ),
            )
            .child(
                div().debug_selector(|| "apply-rerun-failed".into()).child(
                    Button::new("apply-rerun-failed-button")
                        .small()
                        .outline()
                        .label("Re-run failed only")
                        .disabled(!can_rerun || failures == 0)
                        .on_click(cx.listener(|this, _, _, cx| this.rerun_failed(cx))),
                ),
            )
            .child(
                div().debug_selector(|| "apply-apply".into()).child(
                    Button::new("apply-apply-button")
                        .small()
                        .primary()
                        .icon(Icon::new(IconName::Upload).size(u(px(14.))))
                        .label("Apply")
                        .tooltip(
                            blocked
                                .clone()
                                .unwrap_or_else(|| "Apply the plan".to_owned()),
                        )
                        .disabled(blocked.is_some())
                        .on_click(cx.listener(|this, _, _, cx| this.request_apply(cx))),
                ),
            );
        buttons.into_any_element()
    }
}
