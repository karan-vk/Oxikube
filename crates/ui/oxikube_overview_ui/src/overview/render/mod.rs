//! Drawing the overview: the frame here, one file per card. Everything is built from the
//! view-model ([`Shown`](super::view::Shown)) and plain `div`s; the charts are entities that
//! repaint from cached geometry.

mod capacity;
mod graphs;
mod side;
mod tiles;
mod top;

use gpui::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};

use super::view::ClusterOverview;

/// A titled card: the section look every part of the page shares.
pub(super) fn card(id: &'static str, title: &'static str, cx: &App) -> gpui::Stateful<gpui::Div> {
    let tokens = cx.tokens();
    let colors = tokens.colors;
    v_flex()
        .id(id)
        .debug_selector(move || id.to_owned())
        .min_w_0()
        .gap(u(tokens.spacing.md))
        .p(u(tokens.spacing.lg))
        .rounded(u(tokens.radius.md))
        .border_1()
        .border_color(colors.border_variant)
        .bg(colors.surface)
        .child(
            div()
                .text_size(u(tokens.font.small))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text_muted)
                .child(title),
        )
}

/// A muted line of text (an empty or loading state).
pub(super) fn muted(text: impl Into<SharedString>, selector: &'static str, cx: &App) -> AnyElement {
    let tokens = cx.tokens();
    div()
        .debug_selector(move || selector.to_owned())
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .child(text.into())
        .into_any_element()
}

impl ClusterOverview {
    /// The `metrics unavailable: <reason>` line with its way out: a click opens the cluster's
    /// metrics settings.
    pub(super) fn notice(
        &self,
        id: &'static str,
        text: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let tip = SharedString::from(
            "Live usage comes from metrics-server (metrics.k8s.io), graphs from a \
             Prometheus-compatible service. Click for the cluster's metrics settings.",
        );
        h_flex()
            .id(id)
            .debug_selector(move || id.to_owned())
            .min_w_0()
            .gap(u(px(6.)))
            .items_center()
            .text_size(u(tokens.font.small))
            .text_color(colors.warning)
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| this.open_metrics_settings(cx)))
            .child(Icon::new(IconName::TriangleAlert).size(u(px(12.))))
            .child(div().min_w_0().truncate().child(SharedString::from(text)))
            .into_any_element()
    }
}

impl Render for ClusterOverview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let header = h_flex()
            .gap(u(tokens.spacing.lg))
            .items_center()
            .child(
                div()
                    .text_size(u(tokens.font.heading))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Cluster overview"),
            )
            .child(
                div()
                    .debug_selector(|| "overview-context".to_owned())
                    .text_color(colors.text_muted)
                    .child(SharedString::from(self.context.clone())),
            );
        let body = if self.connected {
            let main = v_flex()
                .flex_1()
                .min_w_0()
                .gap(u(tokens.spacing.lg))
                .child(self.capacity(cx))
                .children(self.graphs(cx))
                .child(self.top(cx));
            let side = v_flex()
                .w(u(px(320.)))
                .flex_none()
                .gap(u(tokens.spacing.lg))
                .child(self.info_card(cx))
                .child(self.warnings_card(cx));
            v_flex()
                .gap(u(tokens.spacing.lg))
                .child(self.tiles(cx))
                .child(
                    h_flex()
                        .items_start()
                        .gap(u(tokens.spacing.lg))
                        .child(main)
                        .child(side),
                )
                .into_any_element()
        } else {
            muted("Not connected", "overview-not-connected", cx)
        };
        v_flex()
            .id("cluster-overview")
            .debug_selector(|| "cluster-overview".to_owned())
            .key_context("ClusterOverview")
            .track_focus(&self.focus)
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(
                v_flex()
                    .id("cluster-overview-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .gap(u(tokens.spacing.lg))
                    .p(u(tokens.spacing.xl))
                    .child(header)
                    .child(body),
            )
    }
}
