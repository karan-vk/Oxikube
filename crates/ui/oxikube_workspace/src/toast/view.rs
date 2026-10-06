//! Rendering the toast layer.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use oxikube_ui::{
    ActiveTokens as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    dialog::Cancel,
    layout::{h_flex, v_flex},
    u,
};

use super::{TOAST_KEY_CONTEXT, ToastLayer, ToastLevel, model::Entry};
use crate::{motion, status_bar::STATUS_BAR_HEIGHT};

/// How long a new toast takes to fade in (clamped by the motion policy).
const FADE_IN: Duration = Duration::from_millis(120);
/// Distance from the window edges, unscaled.
const MARGIN: f32 = 12.;
/// Toast width, unscaled.
const WIDTH: f32 = 340.;

impl Render for ToastLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fade = motion::animation_duration(cx, FADE_IN);
        let toasts: Vec<_> = self
            .queue
            .visible
            .iter()
            .map(|entry| self.toast(entry, fade, cx))
            .collect();
        // Absolutely positioned and only as big as its toasts: no layout of the window, and
        // clicks outside the cards reach whatever is below.
        v_flex()
            .id("toast-layer")
            .debug_selector(|| "toast-layer".to_owned())
            .key_context(TOAST_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .absolute()
            .right(u(px(MARGIN)))
            .bottom(u(px(MARGIN + f32::from(STATUS_BAR_HEIGHT))))
            .gap(u(cx.tokens().spacing.md))
            .children(toasts)
    }
}

impl ToastLayer {
    fn toast(&self, entry: &Entry, fade: Option<Duration>, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let id = entry.id;
        let toast = &entry.toast;
        let (icon, accent) = match toast.level {
            ToastLevel::Info => (IconName::Info, colors.info),
            ToastLevel::Success => (IconName::CircleCheck, colors.success),
            ToastLevel::Warning => (IconName::TriangleAlert, colors.warning),
            ToastLevel::Error => (IconName::CircleX, colors.error),
        };
        let selector = format!("toast-{}", id.0);
        let close_selector = format!("toast-{}-close", id.0);
        let focus = self.toast_handles.get(&id).cloned();

        let actions = toast.actions.iter().enumerate().map(|(ix, action)| {
            let handler = action.handler.clone();
            let selector = format!("toast-{}-action-{ix}", id.0);
            let button = Button::new(selector.clone())
                .label(action.label.clone())
                .small()
                .ghost()
                .on_click(cx.listener(move |this, _, window, cx| {
                    handler(window, cx);
                    this.dismiss(id, cx);
                }));
            div().debug_selector(move || selector).child(button)
        });

        let body = v_flex()
            .flex_1()
            .min_w_0()
            .gap(u(tokens.spacing.xs))
            .children(toast.title.clone().map(|title| {
                div()
                    .text_size(u(tokens.font.body))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title)
            }))
            .child(
                div()
                    .text_size(u(tokens.font.body))
                    .text_color(colors.text_muted)
                    .child(toast.message.clone()),
            )
            .child(h_flex().gap(u(tokens.spacing.sm)).children(actions));

        let close = div().debug_selector({
            let selector = close_selector.clone();
            move || selector
        });
        let close = close.child(
            Button::new(close_selector)
                .icon(Icon::new(IconName::X).size(u(px(14.))))
                .xsmall()
                .ghost()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dismiss(id, cx);
                })),
        );

        let card = h_flex()
            .id(("toast", id.0))
            .debug_selector(move || selector)
            .when_some(focus, |this, focus| this.track_focus(&focus))
            .w(u(px(WIDTH)))
            .items_start()
            .gap(u(tokens.spacing.md))
            .p(u(tokens.spacing.lg))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_md()
            .on_action(cx.listener(move |this, _: &Cancel, _, cx| {
                this.dismiss(id, cx);
            }))
            .child(Icon::new(icon).size(u(px(16.))).color(accent))
            .child(body)
            .child(close);

        match fade {
            Some(duration) => card
                .with_animation(
                    ("toast-fade", id.0),
                    Animation::new(duration),
                    |card, delta| card.opacity(delta),
                )
                .into_any_element(),
            None => card.into_any_element(),
        }
    }
}
