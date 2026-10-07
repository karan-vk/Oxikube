//! The banner strip above a terminal's screen (E09-S12): headline, one line of help and the
//! buttons of a [`Banner`].

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, Styled as _, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::error_details::{self, details_box, details_toggle};
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::TerminalView;
use super::lifecycle::{Banner, Tone};

impl TerminalView {
    /// Draws `banner`. The first action is the primary button.
    pub(super) fn banner_strip(&self, banner: Banner, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let (tint, icon) = match banner.tone {
            Tone::Info => (colors.text_muted, IconName::Info),
            Tone::Warning => (colors.warning, IconName::TriangleAlert),
            Tone::Error => (colors.error, IconName::CircleX),
        };
        let view = cx.weak_entity();
        let buttons = banner.actions.iter().enumerate().map(|(index, action)| {
            let action = *action;
            let view = view.clone();
            let id: SharedString = format!("terminal-banner-{action:?}").into();
            let selector = id.clone();
            let mut button =
                Button::new(id)
                    .label(action.label())
                    .small()
                    .on_click(move |_, window, cx| {
                        view.update(cx, |this, cx| this.perform(action, window, cx))
                            .ok();
                    });
            if index == 0 {
                button = button.primary();
            }
            div()
                .debug_selector(move || selector.to_string())
                .child(button)
        });
        let details = banner.details;
        let open = self.banner_details_open;
        let toggle = view.clone();
        let row = h_flex()
            .w_full()
            .items_center()
            .gap(u(tokens.spacing.md))
            .child(Icon::new(icon).size(u(px(16.))).color(tint))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .debug_selector(|| "terminal-banner-headline".to_owned())
                            .font_semibold()
                            .child(banner.headline),
                    )
                    .child(
                        div()
                            .debug_selector(|| "terminal-banner-detail".to_owned())
                            .text_color(colors.text_muted)
                            .text_size(u(tokens.font.small))
                            .child(banner.detail),
                    ),
            )
            .when(details.is_some(), |row| {
                row.child(details_toggle(
                    "terminal-banner-details-toggle",
                    open,
                    move |_, cx| {
                        toggle
                            .update(cx, |this, cx| this.toggle_banner_details(cx))
                            .ok();
                    },
                ))
            })
            .children(buttons);
        v_flex()
            .id("terminal-banner")
            .debug_selector(|| "terminal-banner".to_owned())
            .w_full()
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .bg(tint.opacity(0.14))
            .border_b_1()
            .border_color(tint.opacity(0.5))
            .text_size(u(tokens.font.body))
            .text_color(colors.text)
            .child(row)
            .when_some(details.filter(|_| open), |strip, text| {
                strip.child(details_box(
                    "terminal-banner-details-box",
                    "terminal-banner-details",
                    &text,
                    error_details::SHORT,
                    cx,
                ))
            })
            .into_any_element()
    }
}
