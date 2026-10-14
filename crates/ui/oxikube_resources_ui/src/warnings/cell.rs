//! The badge cell: a warning icon with a tooltip, and a click that shows the object's events.

use std::fmt::Write as _;

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
    let mut line = cell_text(Some(badge));
    if badge.count() > 1 {
        let _ = write!(line, " x{}", badge.count());
    }
    let age = Age::between(badge.last_seen(), now).to_kubectl_string();
    let _ = write!(line, ", {age} ago");
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
        .tooltip(move |window, cx| {
            let tip = tip.clone();
            Tooltip::element(move |_, _| {
                div()
                    .debug_selector(|| "row-warning-tooltip".to_owned())
                    .child(tip.clone())
            })
            .build(window, cx)
        })
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            view.update(cx, |table, cx| table.view_events(key.clone(), cx))
                .ok();
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use jiff::{SignedDuration, Timestamp};

    use super::*;

    fn badge(count: u32, message: &str, last_seen: Timestamp) -> WarningBadge {
        let mut event = oxikube_testkit::event()
            .uid("e")
            .warning()
            .reason("BackOff")
            .message(message)
            .regarding("Pod", Some("x"), "a", "uid-a")
            .build();
        event.count = count;
        event.last_seen = Some(last_seen);
        WarningBadge {
            event: std::sync::Arc::new(event),
        }
    }

    #[test]
    fn the_tooltip_names_the_reason_count_age_message_and_the_click() {
        let now = Timestamp::now();
        let seen = now - SignedDuration::from_mins(2);
        let text = tooltip_text(&badge(3, "Back-off restarting failed container", seen), now);
        assert_eq!(
            text,
            "Warning: BackOff x3, 2m ago\nBack-off restarting failed container\nClick to show the object's events."
        );
    }

    #[test]
    fn a_single_event_has_no_count_and_no_message_no_line() {
        let now = Timestamp::now();
        let text = tooltip_text(&badge(1, "", now - SignedDuration::from_secs(30)), now);
        assert_eq!(
            text,
            "Warning: BackOff, 30s ago\nClick to show the object's events."
        );
    }
}
