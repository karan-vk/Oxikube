//! The right column: the diffs, the preview (objects that change and the manifest), the guard's
//! confirmation, and the upgrade's output log (virtualised).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, uniform_list,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Scrollbar, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, u};

use super::model::Stage;
use super::view::HelmUpgradeView;
use crate::install::log_rows;

impl HelmUpgradeView {
    pub(super) fn side_column(&self, cx: &mut Context<Self>) -> AnyElement {
        let body = match &self.stage {
            Stage::Confirming(_) | Stage::Asking => self.confirm_pane(cx),
            Stage::Upgrading | Stage::Upgraded(_) | Stage::Failed(_) => self.log_pane(cx),
            _ => self.review_pane(cx),
        };
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(body)
            .into_any_element()
    }

    fn confirm_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Stage::Confirming(request) = &self.stage else {
            return div()
                .p(u(tokens.spacing.xl))
                .text_color(colors.text_muted)
                .child("Asking for confirmation...")
                .into_any_element();
        };
        v_flex()
            .debug_selector(|| "helm-upgrade-confirm".into())
            .m(u(tokens.spacing.xl))
            .gap(u(tokens.spacing.md))
            .p(u(tokens.spacing.lg))
            .rounded(u(tokens.radius.md))
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Upgrade this release?"),
            )
            .child(
                div()
                    .debug_selector(|| "helm-upgrade-confirm-summary".into())
                    .child(SharedString::from(request.summary.clone())),
            )
            .child(
                div()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(
                        "Medium risk: the release moves to a new revision; Rollback in its \
                         History returns to this one. Audited.",
                    ),
            )
            .child(
                h_flex()
                    .gap(u(tokens.spacing.sm))
                    .child(
                        div()
                            .debug_selector(|| "helm-upgrade-confirm-yes".into())
                            .child(
                                Button::new("helm-upgrade-confirm-button")
                                    .primary()
                                    .label("Upgrade")
                                    .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "helm-upgrade-confirm-no".into())
                            .child(
                                Button::new("helm-upgrade-decline-button")
                                    .ghost()
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| this.decline(cx))),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn log_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let (status, colour) = match &self.stage {
            Stage::Upgrading => ("Upgrading...".to_owned(), colors.info),
            Stage::Upgraded(message) => (message.clone(), colors.success),
            Stage::Failed(why) => (why.clone(), colors.error),
            _ => (String::new(), colors.text),
        };
        v_flex()
            .debug_selector(|| "helm-upgrade-log-pane".into())
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.sm))
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "helm-upgrade-status".into())
                            .flex_1()
                            .text_color(colour)
                            .child(SharedString::from(status)),
                    )
                    .when(self.stage == Stage::Upgrading, |row| {
                        row.child(
                            div().debug_selector(|| "helm-upgrade-cancel".into()).child(
                                Button::new("helm-upgrade-cancel-button")
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                            ),
                        )
                    }),
            )
            .child(
                div()
                    .debug_selector(|| "helm-upgrade-log".into())
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .font_family(cx.mono_font_family())
                    .text_size(u(tokens.font.small))
                    .child(
                        uniform_list(
                            "helm-upgrade-log-lines",
                            self.log.len(),
                            cx.processor(|this, range, _, cx| log_rows(&this.log, range, cx)),
                        )
                        .track_scroll(&self.log_scroll)
                        .size_full(),
                    )
                    .child(Scrollbar::vertical(&self.log_scroll)),
            )
            .into_any_element()
    }
}
