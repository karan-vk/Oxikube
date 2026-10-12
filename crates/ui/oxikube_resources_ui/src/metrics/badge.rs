//! The `metrics unavailable: <reason>` badge of a Pods or Nodes table's toolbar.

use std::rc::Rc;

use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, px,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_ui::layout::h_flex;
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};
use oxikube_workspace::CommandDispatcher;

/// The badge: a warning icon and `text`. Its tooltip says where to look; a click runs
/// `cluster::ShowMetricsSettings` for `cluster` (the cluster's metrics page, E13-S05). The usage
/// columns stay, reading `–`: absence is shown, never hidden.
pub fn metrics_badge(
    text: SharedString,
    cluster: &ClusterId,
    dispatcher: &Rc<dyn CommandDispatcher>,
    cx: &App,
) -> AnyElement {
    let colors = cx.colors();
    let cluster = cluster.clone();
    let dispatcher = dispatcher.clone();
    let tip = SharedString::from(
        "CPU and memory come from metrics-server (metrics.k8s.io). Click for the cluster's \
         metrics settings.",
    );
    h_flex()
        .id("resource-table-metrics")
        .debug_selector(|| "resource-table-metrics".into())
        .flex_shrink(1.)
        .min_w_0()
        .gap(u(px(4.)))
        .items_center()
        .px(u(px(6.)))
        .h(u(px(20.)))
        .rounded(u(px(10.)))
        .bg(colors.element)
        .border_1()
        .border_color(colors.border_variant)
        .text_size(u(px(11.)))
        .text_color(colors.warning)
        .cursor_pointer()
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(move |_, _, cx| {
            dispatcher.dispatch(
                Command::ClusterShowMetricsSettings {
                    cluster: cluster.clone(),
                },
                cx,
            );
        })
        .child(Icon::new(IconName::TriangleAlert).size(u(px(12.))))
        .child(gpui::div().min_w_0().truncate().child(text))
        .into_any_element()
}
