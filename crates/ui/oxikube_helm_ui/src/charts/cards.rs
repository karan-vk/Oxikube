//! The virtualised list of chart cards.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, hsla, px, uniform_list,
};
use oxikube_ui::layout::{Scrollbar, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, u};

use super::model::Card;
use super::view::HelmChartsView;

/// A card's height at 100 % zoom. Every card has it, which is what lets the list be virtual.
pub const CARD_HEIGHT: f32 = 68.0;

/// The icon placeholder: a rounded tile with the chart's initial in a colour taken from its name.
/// (Remote chart icons are not fetched: showing a catalog must not reach the network by itself.)
pub fn icon_tile(card_initial: char, hue: u16, size: f32) -> AnyElement {
    div()
        .flex_none()
        .size(u(px(size)))
        .rounded(u(px(8.)))
        .bg(hsla(f32::from(hue) / 360.0, 0.45, 0.42, 1.0))
        .text_color(hsla(0., 0., 1., 1.))
        .flex()
        .items_center()
        .justify_center()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(SharedString::from(card_initial.to_string()))
        .into_any_element()
}

impl HelmChartsView {
    /// The line under the search field: searching, the failure, or the empty result.
    pub(super) fn list_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let (id, text, colour) = if let Some(why) = &self.problem {
            ("helm-charts-problem", why.clone(), colors.error)
        } else if self.searching && self.cards.is_empty() {
            (
                "helm-charts-searching",
                "Searching...".to_owned(),
                colors.info,
            )
        } else if self.cards.is_empty() {
            (
                "helm-charts-empty",
                "No charts match. Try another word, or update the repositories.".to_owned(),
                colors.text_muted,
            )
        } else {
            return div().into_any_element();
        };
        div()
            .debug_selector(move || id.into())
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .text_color(colour)
            .child(SharedString::from(text))
            .into_any_element()
    }

    /// The cards, a `uniform_list`: only the rows on screen are built.
    pub(super) fn card_list(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.cards.is_empty() {
            return div().into_any_element();
        }
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .debug_selector(|| "helm-chart-cards".into())
            .child(
                uniform_list(
                    "helm-chart-cards-list",
                    self.cards.len(),
                    cx.processor(|this, range, _, cx| this.card_rows(range, cx)),
                )
                .track_scroll(&self.list_scroll)
                .size_full(),
            )
            .child(Scrollbar::vertical(&self.list_scroll))
            .into_any_element()
    }

    fn card_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        range
            .filter_map(|index| self.cards.get(index).map(|card| (index, card.clone())))
            .map(|(index, card)| self.card(index, &card, cx))
            .collect()
    }

    fn card(&self, index: usize, card: &Card, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let selected = self.selected == Some(index);
        let selector = format!("helm-chart-card:{}", card.reference);
        let version = if card.app_version.is_empty() {
            card.version.clone()
        } else {
            format!("{}  ·  app {}", card.version, card.app_version)
        };
        h_flex()
            .id(("helm-chart-card", index))
            .debug_selector(move || selector)
            .w_full()
            .h(u(px(CARD_HEIGHT)))
            .px(u(tokens.spacing.lg))
            .gap(u(tokens.spacing.md))
            .items_center()
            .border_b_1()
            .border_color(colors.border_variant)
            .cursor_pointer()
            .when(selected, |row| row.bg(colors.element_selected))
            .hover(|row| row.bg(colors.element_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.select(index, cx)))
            .child(icon_tile(card.initial, card.hue, 40.0))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(u(px(2.)))
                    .child(
                        h_flex()
                            .gap(u(tokens.spacing.sm))
                            .items_baseline()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(SharedString::from(card.name.clone())),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(u(tokens.font.small))
                                    .text_color(colors.text_muted)
                                    .child(SharedString::from(card.repo.clone())),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(u(tokens.font.small))
                            .text_color(colors.text_muted)
                            .child(SharedString::from(card.description.clone())),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(SharedString::from(version)),
            )
            .into_any_element()
    }
}
