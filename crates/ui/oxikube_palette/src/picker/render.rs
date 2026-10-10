//! The picker's frame: the query field, the delegate's header, the virtualised list of matches
//! (or the no-match text), the delegate's footer.
//!
//! Written for `oxikube_ui` after reading Zed's `picker/src/render.rs`: the chrome (elevated
//! surface, border, radius, shadow) and the row states (hover, selected) come from the tokens, the
//! rows have one fixed height ([`ROW_HEIGHT`]) so `uniform_list` builds only the visible ones,
//! and the list sizes to its content up to the picker's maximum height.

use std::ops::Range;

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, FontWeight, HighlightStyle, InteractiveElement as _,
    IntoElement, ListSizingBehavior, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
    uniform_list,
};
use oxikube_keymap::KeyContextual as _;
use oxikube_ui::input::Input;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};

use super::{Picker, PickerDelegate, ROW_HEIGHT, fuzzy};

/// The text of a match for [`PickerDelegate::render_match`]: `text` with the characters at
/// `positions` (byte offsets, as in [`fuzzy::StringMatch::positions`]) in the accent colour and
/// bold, muted unless `selected`. Delegates add their own selector or id to the returned box.
pub fn match_label(text: SharedString, positions: &[usize], selected: bool, cx: &App) -> Div {
    let colors = cx.colors();
    let highlight = HighlightStyle {
        color: Some(colors.accent),
        font_weight: Some(FontWeight::BOLD),
        ..HighlightStyle::default()
    };
    div()
        .text_color(if selected {
            colors.text
        } else {
            colors.text_muted
        })
        .child(fuzzy::highlighted_text(text, positions, highlight))
}

impl<D: PickerDelegate> Render for Picker<D> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let header = self.delegate.render_header(window, cx);
        let footer = self.delegate.render_footer(window, cx);
        let body = self.render_body(window, cx);

        v_flex()
            .id("picker")
            .debug_selector(|| "picker".into())
            .key_context(self.key_context())
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::secondary_confirm))
            .w(u(self.width))
            .overflow_hidden()
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .child(
                h_flex()
                    .flex_none()
                    .px(u(tokens.spacing.md))
                    .py(u(tokens.spacing.sm))
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        Input::new(&self.query)
                            .appearance(false)
                            .prefix(Icon::new(IconName::Search).color(colors.text_muted)),
                    ),
            )
            .children(header)
            .child(body)
            .children(footer)
    }
}

impl<D: PickerDelegate> Picker<D> {
    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        if self.delegate.match_count() == 0 {
            let Some(text) = self.delegate.no_matches_text(window, cx) else {
                return div().into_any_element();
            };
            return div()
                .debug_selector(|| "picker-empty".into())
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.md))
                .text_color(tokens.colors.text_muted)
                .child(text)
                .into_any_element();
        }
        v_flex()
            .py(u(tokens.spacing.xs))
            .max_h(u(self.max_height))
            .child(
                uniform_list(
                    "picker-matches",
                    self.delegate.match_count(),
                    cx.processor(|picker, range: Range<usize>, window, cx| {
                        range
                            .map(|ix| picker.render_row(ix, window, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .with_sizing_behavior(ListSizingBehavior::Infer)
                .track_scroll(&self.scroll),
            )
            .into_any_element()
    }

    /// One row: fixed height, the selection and hover backgrounds, the click handler, and the
    /// delegate's content.
    fn render_row(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        // A match the delegate cannot select (a group header) is a label: no hover, no pointer,
        // no click.
        let selectable = self.delegate.can_select(ix, window, cx);
        let selected = selectable && ix == self.delegate.selected_index();
        let content = self.delegate.render_match(ix, selected, window, cx);
        // The outer box spans the list's width (the inset), the inner one is the row itself.
        div()
            .w_full()
            .px(u(tokens.spacing.xs))
            .child(
                h_flex()
                    .id(("picker-row", ix))
                    .debug_selector(move || format!("picker-row-{ix}"))
                    .w_full()
                    .h(u(ROW_HEIGHT))
                    .px(u(tokens.spacing.md))
                    .items_center()
                    .overflow_hidden()
                    .rounded(u(tokens.radius.sm))
                    .when(selectable, |row| row.cursor_pointer())
                    .when(selected, |row| row.bg(colors.element_selected))
                    .when(selectable && !selected, |row| {
                        row.hover(|style| style.bg(colors.element_hover))
                    })
                    .when(selectable, |row| {
                        row.on_click(cx.listener(move |picker, event: &ClickEvent, window, cx| {
                            picker.handle_click(ix, event, window, cx);
                        }))
                    })
                    .children(content),
            )
            .into_any_element()
    }
}
