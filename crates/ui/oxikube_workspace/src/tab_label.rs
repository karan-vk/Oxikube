//! The label drawn inside a tab for an item or a panel: icon, title, dirty dot.

use gpui::{
    App, Div, InteractiveElement as _, ParentElement as _, Styled as _, div, prelude::*, px,
};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, layout::h_flex, u};

use crate::cluster::{BadgeSurface, ClusterBadge, ClusterMark};

/// A tab label. `selector` names it for `debug_bounds` in tests (it is a no-op in release
/// builds); the workspace uses `tab-<title>` for items and `panel-tab-<title>` for panels.
pub(crate) fn tab_label(
    selector: String,
    title: gpui::SharedString,
    icon: Option<IconName>,
    dirty: bool,
    cluster: Option<ClusterMark>,
    cx: &App,
) -> Div {
    let colors = cx.colors();
    let selector_name = selector.clone();
    h_flex()
        .debug_selector(move || selector)
        .gap(u(px(6.)))
        .items_center()
        .when_some(icon, |this, icon| {
            this.child(Icon::new(icon).size(u(px(14.))))
        })
        .when_some(cluster.filter(|mark| !mark.is_plain()), |this, mark| {
            this.child(
                ClusterBadge::new(mark, BadgeSurface::Tab).name(format!("{selector_name}-badge")),
            )
        })
        .child(title)
        .when(dirty, |this| {
            this.child(
                div()
                    .flex_none()
                    .size(u(px(6.)))
                    .rounded_full()
                    .bg(colors.accent),
            )
        })
}
