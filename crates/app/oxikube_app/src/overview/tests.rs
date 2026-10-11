//! The warnings hook: nothing until E14, and a fixed list is capped at what was asked for.

use oxikube_domain::ids::{ClusterId, ContextName};

use super::*;

fn row(object: &str) -> WarningRow {
    WarningRow {
        namespace: Some("shop".into()),
        object: object.to_owned(),
        reason: "BackOff".to_owned(),
        message: "Back-off restarting failed container".to_owned(),
        count: 3,
        last_seen: None,
    }
}

fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::from("kind"))
}

#[test]
fn no_warnings_until_the_events_feed_exists() {
    assert!(NoRecentWarnings.latest(&cluster(), 5).is_empty());
}

#[test]
fn a_fixed_list_is_capped_and_keeps_its_order() {
    let warnings = FixedWarnings(vec![row("Pod/a"), row("Pod/b"), row("Pod/c")]);
    let latest = warnings.latest(&cluster(), 2);
    let objects: Vec<_> = latest.iter().map(|w| w.object.as_str()).collect();
    assert_eq!(objects, ["Pod/a", "Pod/b"]);
    assert_eq!(warnings.latest(&cluster(), 0), []);
}
