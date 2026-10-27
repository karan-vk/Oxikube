//! The frame of the Cloud section: header, notice, the provider cards and the selected provider.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, px, uniform_list,
};
use oxikube_domain::command::CloudRefreshMode;
use oxikube_keymap::KeyContextual as _;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::CloudView;
use crate::cloud::actions::{Cancel, Refresh};

/// Title of the section's empty state.
pub const EMPTY_TITLE: &str = "No cloud CLI found";

/// What the empty state explains, one line each.
pub const EMPTY_STEPS: [&str; 3] = [
    "Oxikube lists the managed clusters of AWS, Google Cloud and Azure by running the CLIs you already use: aws, gcloud and az.",
    "None of them was found. Install one and log in (aws sso login, gcloud auth login or az login), or set its path in settings.",
    "A provider appears here as soon as its CLI is found. Then refresh.",
];

/// Shown while the first answer is awaited.
pub const LOADING_TEXT: &str = "Looking for cloud CLIs...";

impl Render for CloudView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.started {
            self.start(cx);
        }
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = self.body(cx);
        v_flex()
            .id("cloud")
            .debug_selector(|| "cloud".into())
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Refresh, _, cx| {
                this.refresh_all(CloudRefreshMode::Force, cx);
            }))
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(self.header(cx))
            .children(self.notice_line(cx))
            .child(body)
    }
}

impl CloudView {
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let running = self.model.any_running();
        let visible = self.model.visible().count();
        let summary = match visible {
            0 => "no provider found".to_owned(),
            1 => "1 provider".to_owned(),
            n => format!("{n} providers"),
        };
        h_flex()
            .flex_none()
            .gap(u(tokens.spacing.lg))
            .px(u(tokens.spacing.xl))
            .py(u(tokens.spacing.lg))
            .items_center()
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .text_size(u(tokens.font.heading))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Cloud clusters"),
            )
            .child(div().text_color(colors.text_muted).child(summary))
            .child(div().flex_1())
            .child(
                div().debug_selector(|| "cloud-refresh-all".into()).child(
                    Button::new("cloud-refresh-all")
                        .small()
                        .icon(Icon::new(IconName::RefreshCw).size(u(px(14.))))
                        .label("Refresh all")
                        .disabled(running)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_all(CloudRefreshMode::Force, cx);
                        })),
                ),
            )
    }

    /// The line under the header: what the last action did, or why it failed, with the command
    /// that fixes it.
    fn notice_line(&self, cx: &Context<Self>) -> Option<gpui::Div> {
        let notice = self.notice.as_ref()?;
        let tokens = cx.tokens();
        let colour = if notice.error {
            tokens.colors.error
        } else {
            tokens.colors.success
        };
        let copy = notice.command.clone().map(|command| {
            div().debug_selector(|| "cloud-notice-copy".into()).child(
                Button::new("cloud-notice-copy")
                    .small()
                    .ghost()
                    .icon(Icon::new(IconName::Copy).size(u(px(14.))))
                    .label(format!("Copy `{command}`"))
                    .on_click(cx.listener(move |this, _, _, cx| this.copy_command(&command, cx))),
            )
        });
        Some(
            h_flex()
                .flex_none()
                .gap(u(tokens.spacing.md))
                .px(u(tokens.spacing.xl))
                .py(u(tokens.spacing.md))
                .items_center()
                .text_size(u(tokens.font.small))
                .text_color(colour)
                .bg(colour.opacity(0.08))
                .debug_selector(|| "cloud-notice".into())
                .child(
                    Icon::new(if notice.error {
                        IconName::CircleAlert
                    } else {
                        IconName::CircleCheck
                    })
                    .size(u(px(14.))),
                )
                .child(div().flex_1().min_w_0().child(notice.text.clone()))
                .children(copy),
        )
    }

    fn body(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.model.visible().next().is_none() {
            return self.empty(cx).into_any_element();
        }
        let cards = self.cards(cx);
        let detail = self.detail(cx);
        v_flex()
            .flex_1()
            .min_h_0()
            .child(cards)
            .child(detail)
            .into_any_element()
    }

    fn empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let searching = self.model.any_running();
        let title = if searching { LOADING_TEXT } else { EMPTY_TITLE };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(u(tokens.spacing.md))
            .p(u(tokens.spacing.xxl))
            .text_color(tokens.colors.text_muted)
            .debug_selector(move || {
                if searching {
                    "cloud-loading".into()
                } else {
                    "cloud-empty".into()
                }
            })
            .child(if searching {
                Spinner::new().into_any_element()
            } else {
                Icon::new(IconName::Cloud)
                    .size(u(px(32.)))
                    .color(tokens.colors.text_disabled)
                    .into_any_element()
            })
            .child(
                div()
                    .text_size(u(tokens.font.heading))
                    .text_color(tokens.colors.text)
                    .child(title),
            )
            .when(!searching, |column| {
                column.child(
                    v_flex()
                        .max_w(u(px(560.)))
                        .gap(u(tokens.spacing.sm))
                        .children(EMPTY_STEPS.iter().enumerate().map(|(ix, step)| {
                            div()
                                .debug_selector(move || format!("cloud-empty-step-{ix}"))
                                .child(*step)
                        })),
                )
            })
    }

    /// The selected provider: its setup banner, scope filters and cluster list.
    fn detail(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(state) = self.model.selected() else {
            return div().into_any_element();
        };
        let provider = state.provider;
        let count = state.shown().count();
        let banner = self.banner(provider, cx);
        let filters = self.filter_bar(provider, cx);
        let list = if count == 0 {
            self.no_clusters(provider, cx).into_any_element()
        } else {
            uniform_list(
                "cloud-rows",
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|ix| this.render_row(provider, ix, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .into_any_element()
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .children(banner)
            .children(filters)
            .child(list)
            .into_any_element()
    }
}
