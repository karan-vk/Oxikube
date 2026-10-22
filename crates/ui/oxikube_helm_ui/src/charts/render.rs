//! The frame of the charts page: header, guidance for the states without a catalog, and the
//! split of the list and the detail.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, Tokens, u};

use super::model::{Phase, count_line};
use super::view::HelmChartsView;

impl Render for HelmChartsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = match &self.phase {
            Phase::Checking => {
                message(&tokens, "helm-charts-checking", "Looking for helm...").into_any_element()
            }
            Phase::NoHelm => guidance(&tokens),
            Phase::NoRepos => self.no_repos(cx),
            Phase::Failed(why) => self.failed(why.clone(), cx),
            Phase::Ready { .. } => self.split(cx),
        };
        v_flex()
            .id("helm-charts")
            .debug_selector(|| "helm-charts".into())
            .track_focus(&self.focus)
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(self.header(cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}

fn message(tokens: &Tokens, id: &'static str, text: &'static str) -> impl IntoElement {
    div()
        .debug_selector(move || id.into())
        .p(u(tokens.spacing.xl))
        .text_color(tokens.colors.text_muted)
        .child(text)
}

/// What to do when there is no `helm`: the catalog and everything that changes a cluster is hidden.
fn guidance(tokens: &Tokens) -> AnyElement {
    let colors = tokens.colors;
    div()
        .p(u(tokens.spacing.xl))
        .child(
            v_flex()
                .debug_selector(|| "helm-charts-guidance".into())
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
                    "The charts catalog reads chart repositories with the helm command. Install \
                     Helm 3.10 or newer (for example `brew install helm`), or set its location \
                     with the helm.path setting, then restart Oxikube.",
                ))
                .child(
                    div()
                        .text_color(colors.text_muted)
                        .child("Releases, their history and values can still be read without it."),
                ),
        )
        .into_any_element()
}

impl HelmChartsView {
    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let subtitle = match &self.phase {
            Phase::Ready { repos } => count_line(self.cards.len(), *repos),
            _ => "Search the configured chart repositories".to_owned(),
        };
        let can_open = !matches!(self.phase, Phase::NoHelm | Phase::Checking);
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
                    .child("Helm Charts"),
            )
            .child(
                div()
                    .debug_selector(|| "helm-charts-count".into())
                    .text_color(colors.text_muted)
                    .child(SharedString::from(subtitle)),
            )
            .child(div().flex_1())
            .when(can_open, |header| {
                header.child(
                    div().debug_selector(|| "helm-charts-repos".into()).child(
                        Button::new("helm-charts-repos-button")
                            .small()
                            .icon(Icon::new(IconName::Settings).size(u(px(14.))))
                            .label("Repositories")
                            .on_click(cx.listener(|this, _, _, cx| this.open_repos(cx))),
                    ),
                )
            })
            .into_any_element()
    }

    fn no_repos(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let mut chips = h_flex()
            .gap(u(tokens.spacing.sm))
            .items_center()
            .flex_wrap();
        // The user's `helm.default_repos` (hot-reloaded: the view redraws on a settings change).
        let defaults = crate::repos::default_repos(cx);
        for (index, suggestion) in oxikube_app::helm_repos::default_suggestions(&defaults, &[])
            .into_iter()
            .enumerate()
        {
            let name = suggestion.name.clone();
            chips = chips.child(
                div()
                    .debug_selector(move || format!("helm-charts-add:{name}"))
                    .child(
                        Button::new(("helm-charts-add", index))
                            .small()
                            .outline()
                            .label(format!("Add {}", suggestion.name))
                            .tooltip(suggestion.url.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.add_repo(&suggestion.name, &suggestion.url, cx)
                            })),
                    ),
            );
        }
        v_flex()
            .debug_selector(|| "helm-charts-no-repos".into())
            .p(u(tokens.spacing.xl))
            .gap(u(tokens.spacing.md))
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("No chart repositories yet"),
            )
            .child(div().text_color(colors.text_muted).child(
                "Add a repository to browse its charts. Repositories live in Helm's own \
                 configuration.",
            ))
            .child(chips)
            .child(
                h_flex().child(
                    div()
                        .debug_selector(|| "helm-charts-open-repos".into())
                        .child(
                            Button::new("helm-charts-open-repos-button")
                                .small()
                                .primary()
                                .label("Open repositories")
                                .on_click(cx.listener(|this, _, _, cx| this.open_repos(cx))),
                        ),
                ),
            )
            .into_any_element()
    }

    fn failed(&self, why: String, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        v_flex()
            .debug_selector(|| "helm-charts-failed".into())
            .p(u(tokens.spacing.xl))
            .gap(u(tokens.spacing.md))
            .child(
                div()
                    .text_color(tokens.colors.error)
                    .child(SharedString::from(why)),
            )
            .child(
                div().child(
                    Button::new("helm-charts-retry")
                        .small()
                        .label("Retry")
                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                ),
            )
            .into_any_element()
    }

    fn split(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        h_flex()
            .size_full()
            .child(
                v_flex()
                    .w(u(px(420.)))
                    .flex_none()
                    .h_full()
                    .border_r_1()
                    .border_color(colors.border_variant)
                    .child(
                        div()
                            .flex_none()
                            .p(u(tokens.spacing.md))
                            .debug_selector(|| "helm-charts-search".into())
                            .child(Input::new(&self.query)),
                    )
                    .child(self.list_status(cx))
                    .child(self.card_list(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.detail_pane(cx)),
            )
            .into_any_element()
    }
}
