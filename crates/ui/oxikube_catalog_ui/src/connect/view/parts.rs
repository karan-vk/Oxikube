//! The pieces the bodies share.

use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _, div,
    px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

/// The widest a body's text runs, so long lines wrap instead of stretching the card.
pub(super) const CARD_WIDTH: f32 = 560.;

/// A body: centred column, icon, heading, then the caller's children.
pub(super) fn card(
    selector: &'static str,
    icon: IconName,
    icon_colour: gpui::Hsla,
    heading: impl Into<SharedString>,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let tokens = cx.tokens();
    v_flex()
        .id(selector)
        .debug_selector(move || selector.to_owned())
        .size_full()
        .items_center()
        .justify_center()
        .gap(u(tokens.spacing.lg))
        .p(u(tokens.spacing.xxl))
        .text_size(u(tokens.font.body))
        .text_color(tokens.colors.text)
        .child(Icon::new(icon).size(u(px(32.))).color(icon_colour))
        .child(
            div()
                .text_size(u(tokens.font.heading))
                .font_semibold()
                .child(heading.into()),
        )
}

/// A line of muted text under the heading, tagged for tests.
pub(super) fn note(
    selector: impl Into<String>,
    text: impl Into<SharedString>,
    cx: &App,
) -> gpui::Div {
    let selector = selector.into();
    div()
        .debug_selector(move || selector)
        .max_w(u(px(CARD_WIDTH)))
        .text_center()
        .text_color(cx.colors().text_muted)
        .child(text.into())
}

/// A row of buttons.
pub(super) fn actions() -> gpui::Div {
    h_flex().gap_2().items_center().justify_center()
}

/// A button wrapped in a tagged box, so a test finds and clicks it by its selector.
pub(super) fn button(
    selector: &'static str,
    label: &'static str,
    primary: bool,
    enabled: bool,
    on_click: impl Fn(&mut gpui::Window, &mut App) + 'static,
) -> impl IntoElement {
    let button = Button::new(selector)
        .label(label)
        .disabled(!enabled)
        .on_click(move |_, window, cx| on_click(window, cx));
    let button = if primary { button.primary() } else { button };
    div()
        .debug_selector(move || selector.to_owned())
        .child(button)
}

/// A small ghost button, for the secondary actions inside a body.
pub(super) fn link(
    selector: &'static str,
    label: impl Into<SharedString>,
    on_click: impl Fn(&mut gpui::Window, &mut App) + 'static,
) -> impl IntoElement {
    let button = Button::new(selector)
        .label(label.into())
        .small()
        .ghost()
        .on_click(move |_, window, cx| on_click(window, cx));
    div()
        .debug_selector(move || selector.to_owned())
        .child(button)
}
