//! Drawing the Describe tab: the toolbar, the text, the spinner and the error.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    Window, div, px,
};
use oxikube_ports::DescribeSource;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, editor, u};

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
    pub(in crate::detail) fn describe_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let body = self.describe_content(window, cx);
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

    fn describe_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let output = self
            .describe
            .output
            .as_ref()
            .map(|output| (output.text.clone(), output.serial));
        let Some((text, serial)) = output else {
            return match self.describe.state.clone() {
                DescribeState::Failed { kind, message } => {
                    self.describe_failure(kind, &message, cx)
                }
                _ => div()
                    .debug_selector(|| "describe-loading".to_owned())
                    .p(u(tokens.spacing.xl))
                    .child(
                        Spinner::new()
                            .icon(Icon::new(IconName::LoaderCircle))
                            .large()
                            .color(tokens.colors.accent),
                    )
                    .into_any_element(),
            };
        };
        let banner = match self.describe.state.clone() {
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
        let state = match &self.describe.editor {
            Some(state) => state.clone(),
            None => {
                let state = editor::read_only_state(editor::ReadOnly::TEXT, window, cx);
                self.describe.editor = Some(state.clone());
                state
            }
        };
        // The editor is given the text again only when a new answer arrived.
        if self.describe.pushed != Some(serial) {
            editor::set_text(&state, &text, window, cx);
            self.describe.pushed = Some(serial);
        }
        v_flex()
            .size_full()
            .children(banner)
            .child(
                div()
                    .debug_selector(|| "detail-describe-text".to_owned())
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(editor::code_view(&state).h_full()),
            )
            .into_any_element()
    }

    /// The error pane: what went wrong, why for an unsupported kind, and Retry.
    fn describe_failure(
        &self,
        kind: oxikube_domain::ErrorKind,
        message: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let title = match kind {
            oxikube_domain::ErrorKind::Unsupported => "Describe is not available for this kind",
            oxikube_domain::ErrorKind::NotFound => "The object was not found",
            oxikube_domain::ErrorKind::Forbidden => "You may not describe this object",
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
