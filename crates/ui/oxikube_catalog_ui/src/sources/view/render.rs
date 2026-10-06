//! The frame of the sources screen: header, notice, and the list or one of its stand-ins.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, px, uniform_list,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::{SourcesView, empty};
use crate::sources::model::LoadState;

impl Render for SourcesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = self.body(cx);
        v_flex()
            .id("sources")
            .debug_selector(|| "sources".into())
            .track_focus(&self.focus)
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(self.header(cx))
            .children(self.notice(cx))
            .child(body)
    }
}

impl SourcesView {
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let busy = self.busy;
        let summary = self.model.summary();
        let selector = format!("sources-count:{summary}");
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
                    .child("Kubeconfig sources"),
            )
            .child(
                div()
                    .id("sources-count")
                    .debug_selector(move || selector)
                    .text_color(colors.text_muted)
                    .child(summary),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .child(
                        div().debug_selector(|| "sources-add-file".into()).child(
                            Button::new("sources-add-file")
                                .small()
                                .icon(Icon::new(IconName::Plus).size(u(px(14.))))
                                .label("Add file")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.add_file(cx))),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "sources-add-folder".into()).child(
                            Button::new("sources-add-folder")
                                .small()
                                .icon(Icon::new(IconName::Folder).size(u(px(14.))))
                                .label("Add folder")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.add_folder(cx))),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "sources-paste".into()).child(
                            Button::new("sources-paste")
                                .small()
                                .icon(Icon::new(IconName::Clipboard).size(u(px(14.))))
                                .label("Paste kubeconfig")
                                .disabled(busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.open_paste(window, cx)),
                                ),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "sources-reload".into()).child(
                            Button::new("sources-reload")
                                .small()
                                .ghost()
                                .icon(Icon::new(IconName::RefreshCw).size(u(px(14.))))
                                .label("Reload all")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.reload_all(cx))),
                        ),
                    ),
            )
    }

    /// The line under the header: what the last action did, or why it failed.
    fn notice(&self, cx: &Context<Self>) -> Option<gpui::Div> {
        let notice = self.model.notice()?;
        let tokens = cx.tokens();
        let colour = if notice.error {
            tokens.colors.error
        } else {
            tokens.colors.success
        };
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
                .debug_selector(|| "sources-notice".into())
                .child(
                    Icon::new(if notice.error {
                        IconName::CircleAlert
                    } else {
                        IconName::CircleCheck
                    })
                    .size(u(px(14.))),
                )
                .child(div().flex_1().min_w_0().child(notice.text.clone())),
        )
    }

    fn body(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.model.rows().is_empty() {
            return match self.model.load_state() {
                LoadState::Loading => empty::loading(cx).into_any_element(),
                LoadState::Failed(message) => {
                    empty::failed(&message.clone().into(), cx).into_any_element()
                }
                LoadState::Ready => empty::empty(self.busy, cx).into_any_element(),
            };
        }
        uniform_list(
            "sources-rows",
            self.model.rows().len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0()
        .into_any_element()
    }
}
