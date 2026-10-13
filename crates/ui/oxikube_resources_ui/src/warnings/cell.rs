//! The badge cell: a warning icon with a tooltip, and a click that shows the object's events.

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, px,
};
use jiff::Timestamp;
use oxikube_app::WarningBadge;
use oxikube_app::store::ObjectKey;
use oxikube_domain::Age;
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{Icon, IconName, u};

use crate::table::ResourceTable;

/// The tooltip of a badge: `Warning: BackOff x3, 2m ago` and the event's message.
pub(crate) fn tooltip_text(badge: &WarningBadge, now: Timestamp) -> String {
    let mut line = format!("Warning: {}", badge.reason());
    if badge.count() > 1 {
        line.push_str(&format!(" x{}", badge.count()));
    }
    let age = Age::between(badge.last_seen(), now).to_kubectl_string();
    line.push_str(&format!(", {age} ago"));
    if !badge.message().is_empty() {
        line.push('\n');
        line.push_str(badge.message());
    }
    line.push_str("\nClick to show the object's events.");
    line
}

/// The text a copy or an export of the cell gives: the reason, or nothing.
pub(crate) fn cell_text(badge: Option<&WarningBadge>) -> String {
    badge.map_or_else(String::new, |b| format!("Warning: {}", b.reason()))
}

/// The cell of row `row`: the icon in `color` when `badge` is set, else an empty cell.
pub(crate) fn badge_cell(
    badge: Option<WarningBadge>,
    now: Timestamp,
    color: gpui::Hsla,
    row: usize,
    key: ObjectKey,
    view: WeakEntity<ResourceTable>,
) -> AnyElement {
    let Some(badge) = badge else {
        return div().into_any_element();
    };
    let tip = SharedString::from(tooltip_text(&badge, now));
    div()
        .id(("row-warning", row))
        .debug_selector(move || format!("row-warning-{row}"))
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(color)
        .child(Icon::new(IconName::TriangleAlert).size(u(px(14.))))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            view.update(cx, |table, cx| table.view_events(key.clone(), cx))
                .ok();
        })
        .into_any_element()
}
