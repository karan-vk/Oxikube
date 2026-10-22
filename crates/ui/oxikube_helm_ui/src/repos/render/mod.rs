//! The frame of the repositories page.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, Tokens, u};

mod add_form;
mod list;

use super::view::HelmReposView;

impl Render for HelmReposView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let header = self.header(cx);
        let body = if !self.model.checked {
            div()
                .debug_selector(|| "helm-repos-checking".into())
                .text_color(colors.text_muted)
                .child("Looking for helm...")
                .into_any_element()
        } else if self.model.cli_available {
            v_flex()
                .gap(u(tokens.spacing.lg))
                .children(self.banner(&tokens))
                .when(self.form_open, |body| body.child(self.add_form(cx)))
                .children(self.suggestion_chips(cx))
                .child(self.list(cx))
                .into_any_element()
        } else {
            guidance(&tokens)
        };
        v_flex()
            .id("helm-repos")
            .debug_selector(|| "helm-repos".into())
            .track_focus(&self.focus)
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(header)
            .child(
                v_flex()
                    .id("helm-repos-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(u(tokens.spacing.xl))
                    .py(u(tokens.spacing.lg))
                    .child(body),
            )
    }
}

/// What to do when there is no `helm`.
fn guidance(tokens: &Tokens) -> AnyElement {
    let colors = tokens.colors;
    v_flex()
        .debug_selector(|| "helm-repos-guidance".into())
        .gap(u(tokens.spacing.sm))
        .p(u(tokens.spacing.lg))
        .rounded(u(tokens.radius.md))
        .border_1()
        .border_color(colors.border_variant)
        .bg(colors.surface)
        .child(
            h_flex()
                .gap(u(tokens.spacing.md))
                .items_center()
                .text_color(colors.warning)
                .child(
                    Icon::new(IconName::CircleAlert)
                        .size(u(px(16.)))
                        .color(colors.warning),
                )
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child("The helm command was not found"),
                ),
        )
        .child(div().text_color(colors.text_muted).child(
            "Chart repositories are managed with the helm command. Install Helm 3.10 or newer \
             (for example `brew install helm`), or set its location with the helm.path setting, \
             then restart Oxikube.",
        ))
        .child(
            div()
                .text_color(colors.text_muted)
                .child("Releases, their history and values can still be read without it."),
        )
        .into_any_element()
}

impl HelmReposView {
    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let enabled = self.model.cli_available;
        let busy = self.model.busy;
        let can_update = enabled && !busy && !self.model.rows.is_empty();
        h_flex()
            .flex_none()
            .gap(u(tokens.spacing.lg))
            .px(u(tokens.spacing.xl))
            .py(u(tokens.spacing.lg))
            .items_center()
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .text_size(u(tokens.font.heading))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Helm Repositories"),
            )
            .child(
                div()
                    .text_color(colors.text_muted)
                    .child("Stored in Helm's own configuration"),
            )
            .child(div().flex_1())
            .when(enabled, |header| {
                header
                    .child(
                        div()
                            .debug_selector(|| "helm-repos-update-all".into())
                            .child(
                                Button::new("helm-repos-update-all")
                                    .small()
                                    .icon(Icon::new(IconName::RefreshCw).size(u(px(14.))))
                                    .label("Update all")
                                    .tooltip("Refresh the index of every repository")
                                    .disabled(!can_update)
                                    .on_click(cx.listener(|this, _, _, cx| this.update_all(cx))),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "helm-repos-add-toggle".into())
                            .child(
                                Button::new("helm-repos-add-toggle")
                                    .small()
                                    .primary()
                                    .icon(Icon::new(IconName::Plus).size(u(px(14.))))
                                    .label("Add repository")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_form(window, cx)
                                    })),
                            ),
                    )
            })
            .into_any_element()
    }

    /// Progress, the last failure and the last success.
    fn banner(&self, tokens: &Tokens) -> Vec<AnyElement> {
        let colors = tokens.colors;
        let line = |id: &'static str, text: &str, colour: Hsla| {
            div()
                .debug_selector(move || id.into())
                .text_color(colour)
                .child(SharedString::from(text.to_owned()))
                .into_any_element()
        };
        let mut lines = Vec::new();
        if let Some(text) = &self.model.progress {
            lines.push(line("helm-repos-progress", text, colors.info));
        }
        if let Some(text) = &self.model.problem {
            lines.push(line("helm-repos-problem", text, colors.error));
        }
        if let Some(text) = &self.model.notice {
            lines.push(line("helm-repos-notice", text, colors.success));
        }
        lines
    }

    fn suggestion_chips(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.model.suggestions.is_empty() || !self.model.loaded {
            return None;
        }
        let tokens = cx.tokens();
        let mut chips = h_flex()
            .gap(u(tokens.spacing.sm))
            .items_center()
            .flex_wrap()
            .child(
                div()
                    .text_color(tokens.colors.text_muted)
                    .child("Quick add:"),
            );
        for (index, suggestion) in self.model.suggestions.iter().enumerate() {
            chips = chips.child(
                div()
                    .debug_selector(move || format!("helm-repos-suggestion:{index}"))
                    .child(
                        Button::new(("helm-repos-suggestion", index))
                            .xsmall()
                            .outline()
                            .label(suggestion.name.clone())
                            .tooltip(suggestion.url.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.use_suggestion(index, window, cx)
                            })),
                    ),
            );
        }
        Some(chips.into_any_element())
    }
}
