//! The find strip under the tab strip: the field, how many matches, previous and next, close.

use gpui::{
    AnyElement, App, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::Input;
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use crate::detail::tabs::DetailTab;
use crate::detail::view::DetailView;

impl DetailView {
    /// The strip, while the find is open on a tab with text to search.
    pub(in crate::detail) fn find_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.find.open || !matches!(self.tab, DetailTab::Yaml | DetailTab::Describe) {
            return None;
        }
        let input = self.find.input.as_ref()?;
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let label = self.find_label();
        let error = self.find.error.as_ref().map(ToString::to_string);
        let none = self.find.nav.is_empty();
        Some(
            h_flex()
                .id("detail-find")
                .debug_selector(|| "detail-find".to_owned())
                .flex_none()
                .gap(u(tokens.spacing.md))
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.sm))
                .items_center()
                .border_b_1()
                .border_color(colors.border_variant)
                .child(
                    div()
                        .debug_selector(|| "detail-find-input".to_owned())
                        .w(u(px(220.)))
                        .child(Input::new(input).xsmall()),
                )
                .children(label.map(|label| {
                    div()
                        .debug_selector(|| "detail-find-count".to_owned())
                        .text_size(u(tokens.font.small))
                        .text_color(if none {
                            colors.warning
                        } else {
                            colors.text_muted
                        })
                        .child(label)
                }))
                .children(error.map(|message| {
                    div()
                        .debug_selector(|| "detail-find-error".to_owned())
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_size(u(tokens.font.small))
                        .text_color(colors.error)
                        .child(message)
                }))
                .child(div().flex_1())
                .child(icon_button(
                    "detail-find-previous",
                    IconName::ChevronUp,
                    "Previous match (shift-n)",
                    cx.listener(|this, _, _, cx| this.request_previous_match(cx)),
                ))
                .child(icon_button(
                    "detail-find-next",
                    IconName::ChevronDown,
                    "Next match (n)",
                    cx.listener(|this, _, _, cx| this.request_next_match(cx)),
                ))
                .child(icon_button(
                    "detail-find-close",
                    IconName::X,
                    "Close (escape)",
                    cx.listener(|this, _, window, cx| this.close_find(window, cx)),
                ))
                .into_any_element(),
        )
    }
}

/// A ghost icon button of the strip; `id` is also its test selector.
fn icon_button(
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div().debug_selector(|| id.to_owned()).child(
        Button::new(id)
            .xsmall()
            .ghost()
            .icon(Icon::new(icon).size(u(px(12.))))
            .tooltip(tooltip)
            .on_click(on_click),
    )
}
