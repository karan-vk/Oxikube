//! The frame of the catalog: header, column heads, and the list or one of its stand-ins.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, px, uniform_list,
};
use oxikube_keymap::KeyContextual as _;
use oxikube_ui::input::Input;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};

use super::row::COLUMNS;
use super::{CatalogView, empty};
use crate::catalog::actions::{
    ConnectSelected, DisconnectSelected, FocusSearch, SelectFirst, SelectLast, SelectNext,
    SelectPrevious, ToggleFavouriteSelected,
};
use crate::catalog::model::LoadState;

impl Render for CatalogView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = self.body(cx);
        v_flex()
            .id("catalog")
            .debug_selector(|| "catalog".into())
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.select_by(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.select_by(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.select_first(cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.select_last(cx)))
            .on_action(cx.listener(|this, _: &ConnectSelected, _, cx| this.connect_selected(cx)))
            .on_action(
                cx.listener(|this, _: &DisconnectSelected, _, cx| this.disconnect_selected(cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleFavouriteSelected, _, cx| {
                this.toggle_favourite_selected(cx);
            }))
            .on_action(
                cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)),
            )
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(self.header(cx))
            .child(body)
    }
}

impl CatalogView {
    fn header(&self, cx: &Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let count = self.model.count_label();
        let selector = format!("catalog-count:{count}");
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
                    .child("Clusters"),
            )
            .child(
                div()
                    .id("catalog-count")
                    .debug_selector(move || selector)
                    .text_color(colors.text_muted)
                    .child(count),
            )
            .child(div().flex_1())
            .child(
                div().w(u(px(320.))).child(
                    Input::new(&self.search)
                        .prefix(Icon::new(IconName::Search).color(colors.text_muted))
                        .cleanable(true),
                ),
            )
    }

    fn column_heads(&self, cx: &Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        h_flex()
            .flex_none()
            .h(u(px(28.)))
            .px(u(tokens.spacing.xl))
            .items_center()
            .text_color(tokens.colors.text_muted)
            .text_size(u(tokens.font.small))
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .children(COLUMNS.iter().map(|column| column.text(column.title)))
    }

    fn body(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.model.total() == 0 {
            return match self.model.load_state() {
                LoadState::Loading => empty::loading(cx).into_any_element(),
                LoadState::Failed(message) => empty::failed(message, cx).into_any_element(),
                LoadState::Ready => empty::empty(cx).into_any_element(),
            };
        }
        if self.model.visible_len() == 0 {
            return empty::no_match(self.model.query().trim(), cx).into_any_element();
        }
        v_flex()
            .flex_1()
            .min_h_0()
            .child(self.column_heads(cx))
            .child(
                uniform_list(
                    "catalog-rows",
                    self.model.visible_len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .min_h_0(),
            )
            .into_any_element()
    }
}
