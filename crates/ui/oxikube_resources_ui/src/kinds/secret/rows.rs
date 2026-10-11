//! One key of a Secret, as a uniform row.

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, SharedString,
    Styled as _, div, px,
};
use oxikube_app::KeyInfo;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{Icon, IconName, Sizable as _, Tokens, u};

use crate::kinds::key_list::ROW_HEIGHT;

/// What stands where a value is not shown.
pub(super) const DOTS: &str = "••••••••";

/// What a row's value cell says.
pub(super) enum Cell {
    /// Not revealed.
    Masked,
    /// Asked for; the Secret is being read.
    Reading,
    /// A revealed value, as the one line a row shows (made once when the Secret was read).
    Shown(SharedString),
}

/// One key: name, size, the value (dots until revealed) and the eye that reveals it.
pub(super) fn key_row(
    tokens: &Tokens,
    index: usize,
    key: &KeyInfo,
    cell: Cell,
    on_toggle: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let colors = tokens.colors;
    let revealed = matches!(cell, Cell::Shown(_));
    let text = match &cell {
        Cell::Masked => DOTS.to_owned(),
        Cell::Reading => "reading…".to_owned(),
        Cell::Shown(line) => line.to_string(),
    };
    h_flex()
        .id(("secret-key", index))
        .debug_selector(move || format!("secret-key-{index}"))
        .w_full()
        .h(u(px(ROW_HEIGHT)))
        .px(u(tokens.spacing.lg))
        .gap(u(tokens.spacing.md))
        .items_center()
        .child(
            div()
                .flex_none()
                .w(u(px(160.)))
                .truncate()
                .child(key.key.clone()),
        )
        .child(
            div()
                .flex_none()
                .w(u(px(88.)))
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(if key.binary {
                    format!("{} B binary", key.len)
                } else {
                    format!("{} B", key.len)
                }),
        )
        .child(
            div()
                .debug_selector(move || format!("secret-value-{index}"))
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(if revealed {
                    colors.text
                } else {
                    colors.text_muted
                })
                .child(text),
        )
        .child(
            div()
                .debug_selector(move || format!("secret-eye-{index}"))
                .flex_none()
                .child(
                    Button::new(("secret-eye", index))
                        .xsmall()
                        .ghost()
                        .icon(
                            Icon::new(if revealed {
                                IconName::EyeOff
                            } else {
                                IconName::Eye
                            })
                            .size(u(px(14.))),
                        )
                        .tooltip(if revealed {
                            "Hide this value"
                        } else {
                            "Reveal this value"
                        })
                        .on_click(on_toggle),
                ),
        )
        .into_any_element()
}
