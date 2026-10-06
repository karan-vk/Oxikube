//! [`DegradedBanner`]: the strip above a degraded cluster's content.

use gpui::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, px,
};
use oxikube_ui::button::Button;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::model::{ConnectViewModel, DegradedModel};
use super::view::ConnectView;

/// The "some data may be stale" banner of a degraded session, with Retry.
///
/// It draws from the [`ConnectView`] of the same cluster (which follows the session), so the two
/// never disagree, and a retry is the same `cluster::Reconnect`. The cluster tab puts it above
/// the cluster's content, which stays on screen: the data is stale, not gone.
pub struct DegradedBanner {
    view: Entity<ConnectView>,
    _observe: Subscription,
}

impl DegradedBanner {
    /// A banner over `view`.
    pub fn new(view: Entity<ConnectView>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&view, |_, _, cx| cx.notify());
        Self {
            view,
            _observe: observe,
        }
    }
}

impl Render for DegradedBanner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let degraded: Option<DegradedModel> = match self.view.read(cx).model() {
            ConnectViewModel::Degraded(model) => Some(model.clone()),
            _ => None,
        };
        let Some(model) = degraded else {
            // Not degraded (it recovered a moment ago): nothing to draw.
            return div().into_any_element();
        };
        let view = self.view.clone();
        h_flex()
            .id("connect-banner")
            .debug_selector(|| "connect-banner".to_owned())
            .w_full()
            .items_center()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.xl))
            .py(u(tokens.spacing.md))
            .bg(colors.warning.opacity(0.14))
            .border_b_1()
            .border_color(colors.warning.opacity(0.5))
            .text_size(u(tokens.font.body))
            .text_color(colors.text)
            .child(
                Icon::new(IconName::TriangleAlert)
                    .size(u(px(16.)))
                    .color(colors.warning),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .debug_selector(|| "connect-banner-headline".to_owned())
                            .font_semibold()
                            .child(DegradedModel::HEADLINE),
                    )
                    .child(
                        div()
                            .debug_selector(|| "connect-banner-detail".to_owned())
                            .text_color(colors.text_muted)
                            .child(format!("{}: {}", model.title, DegradedModel::DETAIL)),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "connect-banner-retry".to_owned())
                    .child(
                        Button::new("connect-banner-retry")
                            .label("Retry")
                            .small()
                            .on_click(move |_, _, cx| view.update(cx, |this, cx| this.retry(cx))),
                    ),
            )
            .into_any_element()
    }
}
