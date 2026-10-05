//! The label drawn inside a tab for an item or a panel: icon, title, dirty dot.

use gpui::{
    App, Div, InteractiveElement as _, ParentElement as _, Styled as _, div, prelude::*, px,
};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, layout::h_flex, u};

/// A tab label. `selector` names it for `debug_bounds` in tests (it is a no-op in release
/// builds); the workspace uses `tab-<title>` for items and `panel-tab-<title>` for panels.
pub(crate) fn tab_label(
    selector: String,
    title: gpui::SharedString,
    icon: Option<IconName>,
    dirty: bool,
    cx: &App,
) -> Div {
    let colors = cx.colors();
    h_flex()
        .debug_selector(move || selector)
        .gap(u(px(6.)))
        .items_center()
        .when_some(icon, |this, icon| {
            this.child(Icon::new(icon).size(u(px(14.))))
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
