//! The filter chips next to the field (E11-S06): the active filter, with a cross that removes
//! it, and the error of the text being typed.
//!
//! The chip shows what the table is filtered by even when the field has lost the focus or the
//! filter was restored from the last session, so a table that looks short is never a mystery;
//! the cross sends `table::SetFilter` with no text (the same command the jump bar and an agent
//! use), which clears the filter and, with it, the saved one. The error chip says why the text
//! is not a filter; the rows of the last good filter stay on screen meanwhile.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::bar::{FilterBar, FilterBarEvent};

impl FilterBar {
    /// What the chip says: the text of the filter the table shows rows for. `None` without one.
    pub fn chip_label(&self) -> Option<SharedString> {
        (!self.applied.is_empty() && !self.applied_text.is_empty())
            .then(|| SharedString::from(self.applied_text.clone()))
    }

    /// Asks the table to clear the filter (the chip's cross).
    pub fn request_clear(&mut self, cx: &mut Context<Self>) {
        cx.emit(FilterBarEvent::ClearRequested);
    }

    pub(super) fn render_chip(&self, label: SharedString, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        h_flex()
            .id("resource-filter-chip")
            .debug_selector(|| "resource-filter-chip".into())
            .flex_none()
            .max_w(u(px(260.)))
            .h(u(px(20.)))
            .pl(u(px(6.)))
            .gap(u(px(2.)))
            .items_center()
            .rounded(u(px(10.)))
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.element)
            .text_size(u(px(12.)))
            .text_color(colors.text)
            .child(
                Icon::new(IconName::Funnel)
                    .size(u(px(11.)))
                    .color(colors.text_muted),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(label),
            )
            .child(
                div()
                    .debug_selector(|| "resource-filter-chip-clear".into())
                    .child(
                        Button::new("resource-filter-chip-clear")
                            .xsmall()
                            .ghost()
                            .icon(Icon::new(IconName::X).size(u(px(10.))))
                            .tooltip("Clear the filter (escape)")
                            .on_click(cx.listener(|bar, _, _, cx| bar.request_clear(cx))),
                    ),
            )
            .into_any_element()
    }
}

/// The error chip: why the text is not a filter.
pub(super) fn render_error_chip<T: 'static>(
    message: SharedString,
    cx: &mut Context<T>,
) -> AnyElement {
    let colors = cx.colors();
    h_flex()
        .id("resource-filter-error")
        .debug_selector(|| "resource-filter-error".into())
        .min_w_0()
        .max_w(u(px(360.)))
        .h(u(px(20.)))
        .px(u(px(6.)))
        .gap(u(px(4.)))
        .items_center()
        .rounded(u(px(10.)))
        .border_1()
        .border_color(colors.error)
        .text_size(u(px(12.)))
        .text_color(colors.error)
        .child(Icon::new(IconName::CircleAlert).size(u(px(11.))))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(message),
        )
        .into_any_element()
}
