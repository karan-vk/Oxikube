//! The detail pane of the selected chart: header, tabs, README, versions and default values.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px, uniform_list,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Scrollbar, h_flex, v_flex};
use oxikube_ui::markdown::{MarkdownView, is_safe_link};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::cards::icon_tile;
use super::detail::Detail;
use super::model::{Card, Load, Tab};
use super::view::HelmChartsView;

const VERSION_ROW_HEIGHT: f32 = 30.0;

impl HelmChartsView {
    pub(super) fn detail_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let (Some(chart), Some(detail)) = (self.selected_chart(), self.detail.as_ref()) else {
            return div()
                .debug_selector(|| "helm-chart-placeholder".into())
                .p(u(tokens.spacing.xl))
                .text_color(tokens.colors.text_muted)
                .child("Select a chart to read its README, versions and default values.")
                .into_any_element();
        };
        let card = Card::of(chart);
        let colors = tokens.colors;
        let version = chart
            .versions
            .iter()
            .find(|v| v.version == detail.version)
            .unwrap_or_else(|| chart.latest());
        let mut facts = format!("Chart {}", version.version);
        if let Some(app) = &version.app_version {
            facts.push_str(&format!("  ·  App {app}"));
        }
        let n = chart.versions.len();
        facts.push_str(&format!(
            "  ·  {n} version{}",
            if n == 1 { "" } else { "s" }
        ));
        let tabs = Tab::ALL.into_iter().fold(
            h_flex().gap(u(tokens.spacing.sm)).px(u(tokens.spacing.xl)),
            |row, tab| {
                row.child(
                    div()
                        .debug_selector(move || format!("helm-chart-tab:{}", tab.key()))
                        .child({
                            let button = Button::new(("helm-chart-tab", tab as usize))
                                .small()
                                .label(tab.title())
                                .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)));
                            if tab == detail.tab {
                                button.primary()
                            } else {
                                button.ghost()
                            }
                        }),
                )
            },
        );
        v_flex()
            .debug_selector(|| "helm-chart-detail".into())
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .gap(u(tokens.spacing.lg))
                    .px(u(tokens.spacing.xl))
                    .py(u(tokens.spacing.lg))
                    .items_center()
                    .child(icon_tile(card.initial, card.hue, 44.0))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(u(tokens.font.heading))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(SharedString::from(card.reference)),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "helm-chart-facts".into())
                                    .text_color(colors.text_muted)
                                    .child(SharedString::from(facts)),
                            )
                            .child(
                                div()
                                    .text_size(u(tokens.font.small))
                                    .text_color(colors.text_muted)
                                    .child(SharedString::from(version.description.clone())),
                            ),
                    )
                    .child(
                        div().debug_selector(|| "helm-chart-install".into()).child(
                            Button::new("helm-chart-install-button")
                                .primary()
                                .icon(Icon::new(IconName::Download).size(u(px(14.))))
                                .label(format!("Install {}", detail.version))
                                .on_click(cx.listener(|this, _, _, cx| this.install(cx))),
                        ),
                    ),
            )
            .child(tabs)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .mt(u(tokens.spacing.sm))
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .child(self.tab_body(detail, cx)),
            )
            .into_any_element()
    }

    fn tab_body(&self, detail: &Detail, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let note = |id: &'static str, text: String, colour| {
            div()
                .debug_selector(move || id.into())
                .p(u(tokens.spacing.xl))
                .text_color(colour)
                .child(SharedString::from(text))
                .into_any_element()
        };
        let muted = tokens.colors.text_muted;
        match detail.tab {
            Tab::Readme => match &detail.readme {
                Load::Idle | Load::Loading => note(
                    "helm-chart-readme-loading",
                    "Reading the README...".into(),
                    muted,
                ),
                Load::Failed(why) => {
                    note("helm-chart-readme-failed", why.clone(), tokens.colors.error)
                }
                Load::Ready if detail.readme_empty => note(
                    "helm-chart-readme-empty",
                    "This chart has no README.".into(),
                    muted,
                ),
                Load::Ready => div()
                    .debug_selector(|| "helm-chart-readme".into())
                    .size_full()
                    .px(u(tokens.spacing.xl))
                    .child(
                        MarkdownView::new(&detail.markdown)
                            .scrollable(true)
                            .selectable(true)
                            .on_link_click(|url, _, _, cx| {
                                // Only web and mail links leave the app, and only by a click.
                                if is_safe_link(url) {
                                    cx.open_url(url);
                                }
                            })
                            .size_full(),
                    )
                    .into_any_element(),
            },
            Tab::Values => match &detail.values {
                Load::Idle | Load::Loading => note(
                    "helm-chart-values-loading",
                    "Reading the default values...".into(),
                    muted,
                ),
                Load::Failed(why) => {
                    note("helm-chart-values-failed", why.clone(), tokens.colors.error)
                }
                Load::Ready if detail.values_empty => note(
                    "helm-chart-values-empty",
                    "This chart has no default values.".into(),
                    muted,
                ),
                Load::Ready => div()
                    .debug_selector(|| "helm-chart-values".into())
                    .size_full()
                    .child(detail.code.clone())
                    .into_any_element(),
            },
            Tab::Versions => self.versions(detail, cx),
        }
    }

    fn versions(&self, detail: &Detail, cx: &mut Context<Self>) -> AnyElement {
        let count = self.selected_chart().map_or(0, |c| c.versions.len());
        div()
            .relative()
            .size_full()
            .debug_selector(|| "helm-chart-versions".into())
            .child(
                uniform_list(
                    "helm-chart-versions-list",
                    count,
                    cx.processor(|this, range, _, cx| this.version_rows(range, cx)),
                )
                .track_scroll(&detail.versions_scroll)
                .size_full(),
            )
            .child(Scrollbar::vertical(&detail.versions_scroll))
            .into_any_element()
    }

    fn version_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let (Some(chart), Some(detail)) = (self.selected_chart(), self.detail.as_ref()) else {
            return Vec::new();
        };
        range
            .filter_map(|index| {
                let v = chart.versions.get(index)?;
                let current = v.version == detail.version;
                let version = v.version.clone();
                let selector = format!("helm-chart-version:{}", v.version);
                let text = match &v.app_version {
                    Some(app) => format!("app {app}"),
                    None => String::new(),
                };
                Some(
                    h_flex()
                        .id(("helm-chart-version", index))
                        .debug_selector(move || selector)
                        .h(u(px(VERSION_ROW_HEIGHT)))
                        .px(u(tokens.spacing.xl))
                        .gap(u(tokens.spacing.lg))
                        .items_center()
                        .cursor_pointer()
                        .when(current, |row| row.bg(colors.element_selected))
                        .hover(|row| row.bg(colors.element_hover))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_version(&version, cx)))
                        .child(
                            div()
                                .w(u(px(160.)))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(SharedString::from(v.version.clone())),
                        )
                        .child(
                            div()
                                .text_color(colors.text_muted)
                                .child(SharedString::from(text)),
                        )
                        .when(current, |row| {
                            row.child(div().text_color(colors.accent).child("selected"))
                        })
                        .into_any_element(),
                )
            })
            .collect()
    }
}
