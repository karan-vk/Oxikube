//! The container picker and the range dropdown.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div,
};
use oxikube_domain::log::LogRange;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::menu::DropdownMenu as _;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::overflow::menu_row;
use crate::view::LogView;
use crate::view::containers::container_label;

/// What the range dropdown's button and rows say: `tail 1000` (the lines the tail reads), `head`,
/// `since 5m`.
pub(crate) fn range_label(range: LogRange, tail_lines: u32) -> String {
    match range {
        LogRange::Tail => format!("tail {tail_lines}"),
        LogRange::Head => "head".to_owned(),
        since => format!("since {}", since.label()),
    }
}

impl LogView {
    /// The lines the tail range reads in this view.
    fn tail_length(&self) -> u32 {
        self.options.tail_lines.unwrap_or(self.options.default_tail)
    }

    /// What the container picker says (`main (1/2)`).
    pub(crate) fn container_picker_label(&self) -> String {
        container_label(self.options.container.as_deref(), &self.containers)
    }

    /// What the range dropdown's button says (`tail 1000`, `since 5m`, `head`).
    pub(crate) fn range_button_label(&self) -> String {
        range_label(self.options.range, self.tail_length())
    }

    /// The container picker. With several containers it is a dropdown button with a caret and
    /// the container's place (`main (1/2)`); with one (or none read yet) it is a plain label,
    /// since there is nothing to pick.
    pub(crate) fn container_selector(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.options.container.clone();
        let label = self.container_picker_label();
        if self.containers.len() < 2 {
            let tokens = cx.tokens();
            return div()
                .flex_none()
                .debug_selector(|| "log-container".into())
                .px(u(tokens.spacing.sm))
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text)
                .child(label)
                .into_any_element();
        }
        let choices: Vec<(String, String, bool)> = self
            .containers
            .iter()
            .map(|c| (c.name.to_string(), c.label(), c.crash_looping))
            .collect();
        let view = cx.entity().downgrade();
        let button = Button::new("log-container")
            .label(label)
            .ghost()
            .xsmall()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for (name, label, crashing) in &choices {
                    let (view, name) = (view.clone(), name.clone());
                    let checked = current.as_deref() == Some(name.as_str());
                    let label = if *crashing {
                        format!("{label} (crash-looping)")
                    } else {
                        label.clone()
                    };
                    menu = menu.item(
                        menu_row(format!("log-container:{name}"), label)
                            .checked(checked)
                            .on_click(move |_, _, cx| {
                                view.update(cx, |view, cx| view.request_container(&name, cx))
                                    .ok();
                            }),
                    );
                }
                menu
            });
        div()
            .flex_none()
            .debug_selector(|| "log-container".into())
            .child(button)
            .into_any_element()
    }

    /// The range dropdown: the current range on the button, every preset in the menu.
    pub(crate) fn range_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let tail = self.tail_length();
        let current = self.options.range;
        let view = cx.entity().downgrade();
        let button = Button::new("log-range")
            .label(self.range_button_label())
            .ghost()
            .xsmall()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for range in LogRange::ALL {
                    let view = view.clone();
                    menu = menu.item(
                        menu_row(
                            format!("log-range-{}", range.label()),
                            range_label(range, tail),
                        )
                        .checked(range == current)
                        .on_click(move |_, _, cx| {
                            view.update(cx, |view, cx| view.request_range(range, cx))
                                .ok();
                        }),
                    );
                }
                menu
            });
        div()
            .flex_none()
            .debug_selector(|| "log-range".into())
            .child(button)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_range_says_what_it_reads() {
        assert_eq!(range_label(LogRange::Tail, 1000), "tail 1000");
        assert_eq!(range_label(LogRange::Head, 1000), "head");
        assert_eq!(range_label(LogRange::Last5m, 1000), "since 5m");
        assert_eq!(range_label(LogRange::Last1h, 1000), "since 1h");
    }
}
