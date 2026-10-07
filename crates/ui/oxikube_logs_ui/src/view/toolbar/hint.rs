//! The hint under the toolbar of a crash-looping container: the log of its current instance is
//! often empty or a few lines of start-up, and "Previous" holds the last crash.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use crate::view::LogView;

impl LogView {
    /// The strip that says the container is crash-looping and "Previous" holds its last crash,
    /// with a button that sends `logs::TogglePrevious`; `None` otherwise (and once the previous
    /// instance is shown).
    pub(crate) fn crash_hint(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.shows_crash_hint() {
            return None;
        }
        let tokens = cx.tokens();
        let button = Button::new("log-crash-previous")
            .label("Show previous")
            .primary()
            .xsmall()
            .on_click(cx.listener(|view, _, _, cx| view.request_previous(cx)));
        Some(
            h_flex()
                .id("log-crash-hint")
                .debug_selector(|| "log-crash-hint".into())
                .flex_none()
                .items_center()
                .gap(u(tokens.spacing.md))
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.sm))
                .bg(tokens.colors.surface)
                .border_b_1()
                .border_color(tokens.colors.border_variant)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(u(tokens.font.small))
                        .text_color(tokens.colors.warning)
                        .child(
                            "This container is crash-looping. Previous holds the log of its last crash.",
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .debug_selector(|| "log-crash-previous".into())
                        .child(button),
                )
                .into_any_element(),
        )
    }
}
