//! The search bar: the field, the case / inverse / filter toggles, previous and next, the count
//! (or the pattern's error) and close. Every control sends its `logs::*` command, like the keys.

use gpui::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Styled as _, Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::{Input, InputEvent, InputState};
use oxikube_ui::layout::{Selectable as _, h_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::SearchMode;
use super::text::status_text;
use crate::LogView;

impl LogView {
    /// Makes the field the first time the bar is shown (a field needs a window), with the text
    /// the search already has (a restored one).
    pub(crate) fn ensure_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(input) = &self.search.input {
            return input.clone();
        }
        let text = self.search.state.text().to_owned();
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Search (regex)");
            input.set_value(text, window, cx);
            input
        });
        self.search.subscription = Some(cx.subscribe_in(&input, window, Self::on_search_input));
        self.search.input = Some(input.clone());
        input
    }

    fn on_search_input(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let text = input.read(cx).value().to_string();
                if text != self.search.state.text() {
                    self.edit_search(&text, cx);
                }
            }
            InputEvent::PressEnter { shift, .. } => {
                if *shift {
                    self.request_previous_match(cx);
                } else {
                    self.request_next_match(cx);
                }
            }
            InputEvent::Focus => self.set_search_editing(true, cx),
            InputEvent::Blur => self.set_search_editing(false, cx),
        }
    }

    fn set_search_editing(&mut self, editing: bool, cx: &mut Context<Self>) {
        if self.search.editing != editing {
            self.search.editing = editing;
            // The key context says `Editing` meanwhile, so bare keys are text.
            cx.notify();
        }
    }

    /// The bar, when it is open.
    pub(crate) fn search_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.search.state.is_open() {
            return None;
        }
        let input = self.ensure_search_input(window, cx);
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let state = &self.search.state;
        let status: SharedString = status_text(state, self.search_counts()).into();
        let failed = state.error().is_some();
        let (case, inverse) = (state.case_sensitive(), state.inverse());
        let filtering = state.mode() == SearchMode::Filter;
        Some(
            h_flex()
                .id("log-search")
                .debug_selector(|| "log-search".into())
                .flex_none()
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.sm))
                .items_center()
                .border_b_1()
                .border_color(colors.border_variant)
                .child(Icon::new(IconName::Search).size(u(px(14.))))
                .child(
                    div()
                        .debug_selector(|| "log-search-input".into())
                        .w(u(px(280.)))
                        .child(Input::new(&input).xsmall()),
                )
                .child(
                    self.search_toggle("log-search-case", "Aa", case, cx, |v, cx| {
                        v.request_toggle_case(cx)
                    }),
                )
                .child(
                    self.search_toggle("log-search-inverse", "Not", inverse, cx, |v, cx| {
                        v.request_toggle_inverse(cx)
                    }),
                )
                .child(
                    self.search_toggle("log-search-filter", "Filter", filtering, cx, |v, cx| {
                        v.request_toggle_filter_mode(cx)
                    }),
                )
                .child(
                    self.search_step("log-search-prev", IconName::ArrowUp, cx, |v, cx| {
                        v.request_previous_match(cx)
                    }),
                )
                .child(
                    self.search_step("log-search-next", IconName::ArrowDown, cx, |v, cx| {
                        v.request_next_match(cx)
                    }),
                )
                .child(
                    div()
                        .debug_selector(|| "log-search-status".into())
                        .max_w(u(px(420.)))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(u(tokens.font.small))
                        .text_color(if failed {
                            colors.error
                        } else {
                            colors.text_muted
                        })
                        .child(status),
                )
                .child(div().flex_1())
                .child(
                    self.search_step("log-search-close", IconName::X, cx, |v, cx| {
                        v.request_close_search(cx)
                    }),
                )
                .into_any_element(),
        )
    }

    fn search_toggle(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        request: fn(&mut LogView, &mut Context<LogView>),
    ) -> AnyElement {
        div()
            .debug_selector(move || id.to_owned())
            .child(
                Button::new(id)
                    .label(label)
                    .ghost()
                    .xsmall()
                    .selected(on)
                    .on_click(cx.listener(move |view, _, _, cx| request(view, cx))),
            )
            .into_any_element()
    }

    fn search_step(
        &self,
        id: &'static str,
        icon: IconName,
        cx: &mut Context<Self>,
        request: fn(&mut LogView, &mut Context<LogView>),
    ) -> AnyElement {
        div()
            .debug_selector(move || id.to_owned())
            .child(
                Button::new(id)
                    .icon(Icon::new(icon).size(u(px(14.))))
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(move |view, _, _, cx| request(view, cx))),
            )
            .into_any_element()
    }
}
