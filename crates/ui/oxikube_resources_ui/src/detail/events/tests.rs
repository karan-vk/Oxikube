use std::sync::Arc;

use oxikube_domain::event::EventType;
use oxikube_testkit::event;

use super::*;

#[test]
fn a_row_carries_the_columns_of_the_events_view() {
    let warning = event()
        .uid("e")
        .warning()
        .reason("BackOff")
        .message("Back-off restarting failed container")
        .count(7)
        .last_seen("2026-01-01T00:10:00Z")
        .build();
    let row = EventRow::from(&warning);
    assert_eq!(row.kind, EventType::Warning);
    assert_eq!(row.type_label(), "Warning");
    assert_eq!(&*row.reason, "BackOff");
    assert_eq!(row.message, "Back-off restarting failed container");
    assert_eq!(row.count, 7);
    assert_eq!(row.last_seen, Some("2026-01-01T00:10:00Z".parse().unwrap()));
    assert_eq!(row.source.as_deref(), Some("default-scheduler"));
}

#[test]
fn rows_keep_the_feeds_order() {
    let events: Vec<_> = ["newest", "middle", "oldest"]
        .into_iter()
        .map(|reason| Arc::new(event().uid(reason).reason(reason).build()))
        .collect();
    let rows = rows_of(&events);
    let reasons: Vec<_> = rows.iter().map(|r| &*r.reason).collect();
    assert_eq!(reasons, ["newest", "middle", "oldest"]);
}

#[test]
fn a_normal_event_says_normal() {
    let row = EventRow::from(&event().build());
    assert_eq!(row.type_label(), "Normal");
}
