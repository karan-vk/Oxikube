//! One key of a ConfigMap, as a uniform row.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_domain::view::{BinaryEntry, ConfigEntry};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{Icon, IconName, Tokens, u};

use crate::kinds::key_list::ROW_HEIGHT;

/// The width of the key column.
const KEY_WIDTH: f32 = 160.;

/// "1.2 KB", "340 B".
pub(super) fn size_text(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.),
    }
}

/// A text key: chevron, name, language, size and the first line of the value. `selected` is the
/// key whose preview is open.
pub(super) fn entry_row(
    tokens: &Tokens,
    index: usize,
    entry: &ConfigEntry,
    selected: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let colors = tokens.colors;
    h_flex()
        .id(("configmap-key", index))
        .debug_selector(move || format!("configmap-key-{index}"))
        .w_full()
        .h(u(px(ROW_HEIGHT)))
        .px(u(tokens.spacing.lg))
        .gap(u(tokens.spacing.md))
        .items_center()
        .cursor_pointer()
        .when(selected, |row| row.bg(colors.element_selected))
        .hover(|style| style.bg(colors.element_hover))
        .on_click(on_click)
        .child(
            Icon::new(if selected {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .size(u(px(12.)))
            .color(colors.text_muted),
        )
        .child(
            div()
                .flex_none()
                .w(u(px(KEY_WIDTH)))
                .truncate()
                .child(entry.key.to_string()),
        )
        .child(
            div()
                .flex_none()
                .w(u(px(64.)))
                .truncate()
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(entry.language.label()),
        )
        .child(
            div()
                .flex_none()
                .w(u(px(64.)))
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(size_text(entry.size)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(entry.summary.clone()),
        )
        .into_any_element()
}

/// A `binaryData` key: its name and size, nothing to expand.
pub(super) fn binary_row(tokens: &Tokens, index: usize, entry: &BinaryEntry) -> AnyElement {
    let colors = tokens.colors;
    h_flex()
        .id(("configmap-binary", index))
        .debug_selector(move || format!("configmap-binary-{index}"))
        .w_full()
        .h(u(px(ROW_HEIGHT)))
        .px(u(tokens.spacing.lg))
        .gap(u(tokens.spacing.md))
        .items_center()
        .child(div().flex_none().w(u(px(12.))))
        .child(
            div()
                .flex_none()
                .w(u(px(KEY_WIDTH)))
                .truncate()
                .child(entry.key.to_string()),
        )
        .child(
            div()
                .flex_none()
                .w(u(px(64.)))
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child("binary"),
        )
        .child(
            div()
                .flex_none()
                .w(u(px(64.)))
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(size_text(entry.size)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::size_text;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size_text(0), "0 B");
        assert_eq!(size_text(1023), "1023 B");
        assert_eq!(size_text(1536), "1.5 KB");
        assert_eq!(size_text(3 * 1024 * 1024), "3.0 MB");
    }
}
