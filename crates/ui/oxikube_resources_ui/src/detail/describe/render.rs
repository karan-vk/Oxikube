//! Drawing the Describe tab: the toolbar, the text (a read-only code view), the spinner and the
//! error.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, prelude::FluentBuilder as _, px,
};
use oxikube_domain::ErrorKind;
use oxikube_ports::DescribeSource;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::tab::DescribeState;
use crate::detail::view::DetailView;
use crate::table::ToneColors;

/// Which backend rendered the text, for the toolbar.
fn source_label(source: DescribeSource) -> &'static str {
    match source {
        DescribeSource::Native => "Rendered natively (deskribe)",
        DescribeSource::KubectlFallback => "Rendered by kubectl describe",
    }
}

impl DetailView {
    /// The Describe tab.
    pub(in crate::detail) fn describe_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let body = self.describe_content(cx);
        v_flex()
            .id("detail-describe")
            .debug_selector(|| "detail-describe".to_owned())
            .size_full()
            .child(self.describe_toolbar(cx))
            .child(div().flex_1().min_h_0().min_w_0().child(body))
            .into_any_element()
    }

    fn describe_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let loading = matches!(self.describe.state, DescribeState::Loading);
        let label = self
            .describe
            .output
            .as_ref()
            .map(|output| source_label(output.source))
            .unwrap_or_default();
        h_flex()
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .text_size(u(tokens.font.small))
            .text_color(tokens.colors.text_muted)
            .child(
                div()
                    .debug_selector(|| "describe-source".to_owned())
                    .flex_1()
                    .child(label),
            )
            .child(
                div()
                    .debug_selector(|| "describe-refresh".to_owned())
                    .child(
                        Button::new("describe-refresh")
                            .xsmall()
                            .ghost()
                            .icon(Icon::new(IconName::RefreshCw).size(u(px(14.))))
                            .loading(loading)
                            .tooltip("Refresh")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.request_refresh_describe(cx)),
                            ),
                    ),
            )
    }

    fn describe_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let spinner = || {
            div()
                .debug_selector(|| "describe-loading".to_owned())
                .p(u(tokens.spacing.xl))
                .child(
                    Spinner::new()
                        .icon(Icon::new(IconName::LoaderCircle))
                        .large()
                        .color(tokens.colors.accent),
                )
        };
        // The view is made with the first answer, so it exists once there is a text.
        let Some(view) = self.describe.view.clone() else {
            return match &self.describe.state {
                DescribeState::Failed { kind, message } => {
                    self.describe_failure(*kind, message, cx)
                }
                _ => spinner().into_any_element(),
            };
        };
        let banner = match &self.describe.state {
            DescribeState::Failed { message, .. } => Some(
                div()
                    .debug_selector(|| "describe-refresh-error".to_owned())
                    .flex_none()
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.md))
                    .bg(tones.error.opacity(0.14))
                    .text_color(tones.error)
                    .text_size(u(tokens.font.small))
                    .child(format!("Refreshing failed: {message}")),
            ),
            _ => None,
        };
        // The view lays the text out off the UI thread; the spinner covers it until its first
        // text is on screen.
        let ready = view.read(cx).is_ready();
        v_flex()
            .size_full()
            .children(banner)
            .child(
                div()
                    .debug_selector(|| "detail-describe-text".to_owned())
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(view)
                    .when(!ready, |this| {
                        this.child(spinner().absolute().inset_0().bg(tokens.colors.surface))
                    }),
            )
            .into_any_element()
    }

    /// The error pane: what went wrong, why for an unsupported kind, and Retry.
    fn describe_failure(
        &self,
        kind: ErrorKind,
        message: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let title = match kind {
            ErrorKind::Unsupported => "Describe is not available for this kind",
            ErrorKind::NotFound => "The object was not found",
            ErrorKind::Forbidden => "You may not describe this object",
            _ => "Describe failed",
        };
        v_flex()
            .debug_selector(|| "describe-error".to_owned())
            .gap(u(tokens.spacing.md))
            .p(u(tokens.spacing.xl))
            .child(div().text_color(tones.error).child(title))
            .child(
                div()
                    .debug_selector(|| "describe-error-message".to_owned())
                    .text_size(u(tokens.font.small))
                    .text_color(tokens.colors.text_muted)
                    .child(message.to_owned()),
            )
            .child(
                div().debug_selector(|| "describe-retry".to_owned()).child(
                    Button::new("describe-retry")
                        .xsmall()
                        .label("Retry")
                        .on_click(cx.listener(|this, _, _, cx| this.request_refresh_describe(cx))),
                ),
            )
            .into_any_element()
    }
}
