//! The right column: the preview (manifest and verdict), the guard's confirmation, and the
//! install's output log (virtualised).

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px, uniform_list,
};
use oxikube_domain::helm::OutputKind;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Scrollbar, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, u};

use super::model::Stage;
use super::view::HelmInstallView;

/// A log line's height at 100 % zoom (uniform, so the list is virtual).
const LOG_ROW_HEIGHT: f32 = 18.0;

impl HelmInstallView {
    pub(super) fn side_column(&self, cx: &mut Context<Self>) -> AnyElement {
        let body = match &self.stage {
            Stage::Confirming(_) | Stage::Asking => self.confirm_pane(cx),
            Stage::Installing | Stage::Installed(_) | Stage::Failed(_) => self.log_pane(cx),
            _ => self.preview_pane(cx),
        };
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(body)
            .into_any_element()
    }

    fn preview_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let fresh = self.fresh_preview(cx).is_some();
        let headline = match (&self.shown, &self.preview_error, &self.stage) {
            (_, _, Stage::Previewing) => (
                "Rendering the chart and asking the server...".to_owned(),
                colors.info,
            ),
            (_, Some(_), _) => ("The preview failed".to_owned(), colors.error),
            (Some(shown), _, _) if !fresh => (
                format!(
                    "{} (out of date: preview again)",
                    shown.preview.summary.describe()
                ),
                colors.warning,
            ),
            (Some(shown), _, _) => (shown.preview.summary.describe(), colors.text),
            (None, None, _) => (
                "Preview renders the chart with your values and has the server check it; \
                 nothing is installed."
                    .to_owned(),
                colors.text_muted,
            ),
        };
        let server = self
            .shown
            .as_ref()
            .and_then(|s| s.preview.server_error.clone());
        v_flex()
            .debug_selector(|| "helm-install-preview-pane".into())
            .size_full()
            .child(
                v_flex()
                    .flex_none()
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.sm))
                    .gap(u(px(2.)))
                    .child(div().font_weight(gpui::FontWeight::MEDIUM).child("Preview"))
                    .child(
                        div()
                            .debug_selector(|| "helm-install-summary".into())
                            .text_color(headline.1)
                            .child(SharedString::from(headline.0)),
                    )
                    .when_some(self.preview_error.clone(), |pane, why| {
                        pane.child(
                            div()
                                .debug_selector(|| "helm-install-preview-error".into())
                                .text_size(u(tokens.font.small))
                                .text_color(colors.error)
                                .child(SharedString::from(why)),
                        )
                    })
                    .when_some(server, |pane, why| {
                        pane.child(
                            div()
                                .debug_selector(|| "helm-install-server-error".into())
                                .text_size(u(tokens.font.small))
                                .text_color(colors.error)
                                .child(SharedString::from(format!("The server refused it: {why}"))),
                        )
                    }),
            )
            .when(self.shown.is_some(), |pane| {
                pane.child(
                    div()
                        .debug_selector(|| "helm-install-manifest".into())
                        .flex_1()
                        .min_h_0()
                        .border_t_1()
                        .border_color(colors.border_variant)
                        .child(oxikube_ui::editor::code_view(&self.manifest).size_full()),
                )
            })
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
            .debug_selector(|| "helm-install-confirm".into())
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
                    .child("Install this release?"),
            )
            .child(
                div()
                    .debug_selector(|| "helm-install-confirm-summary".into())
                    .child(SharedString::from(request.summary.clone())),
            )
            .child(
                div()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child("Low risk: a new release, removable with Uninstall. Audited."),
            )
            .child(
                h_flex()
                    .gap(u(tokens.spacing.sm))
                    .child(
                        div()
                            .debug_selector(|| "helm-install-confirm-yes".into())
                            .child(
                                Button::new("helm-install-confirm-button")
                                    .primary()
                                    .label("Install")
                                    .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "helm-install-confirm-no".into())
                            .child(
                                Button::new("helm-install-decline-button")
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
            Stage::Installing => ("Installing...".to_owned(), colors.info),
            Stage::Installed(message) => (message.clone(), colors.success),
            Stage::Failed(why) => (why.clone(), colors.error),
            _ => (String::new(), colors.text),
        };
        v_flex()
            .debug_selector(|| "helm-install-log-pane".into())
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
                            .debug_selector(|| "helm-install-status".into())
                            .flex_1()
                            .text_color(colour)
                            .child(SharedString::from(status)),
                    )
                    .when(self.stage == Stage::Installing, |row| {
                        row.child(
                            div().debug_selector(|| "helm-install-cancel".into()).child(
                                Button::new("helm-install-cancel-button")
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                            ),
                        )
                    }),
            )
            .child(
                div()
                    .debug_selector(|| "helm-install-log".into())
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .font_family(cx.mono_font_family())
                    .text_size(u(tokens.font.small))
                    .child(
                        uniform_list(
                            "helm-install-log-lines",
                            self.log.len(),
                            cx.processor(|this, range, _, cx| this.log_rows(range, cx)),
                        )
                        .track_scroll(&self.log_scroll)
                        .size_full(),
                    )
                    .child(Scrollbar::vertical(&self.log_scroll)),
            )
            .into_any_element()
    }

    fn log_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        range
            .filter_map(|i| self.log.get(i))
            .map(|line| {
                let colour = match line.kind {
                    OutputKind::Stdout => tokens.colors.text,
                    OutputKind::Stderr => tokens.colors.warning,
                };
                div()
                    .h(u(px(LOG_ROW_HEIGHT)))
                    .px(u(tokens.spacing.lg))
                    .truncate()
                    .text_color(colour)
                    .child(SharedString::from(line.text.clone()))
                    .into_any_element()
            })
            .collect()
    }
}
