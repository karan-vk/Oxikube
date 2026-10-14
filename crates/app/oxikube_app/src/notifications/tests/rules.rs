//! Which events notify, and the input an event becomes.

use std::sync::Arc;

use oxikube_domain::event::EventType;
use oxikube_ports::NotificationLevel;
use oxikube_testkit::event;

use super::{cluster, other, warning};
use crate::notifications::{ClusterRules, NotificationRules};

#[test]
fn a_warning_about_a_watched_kind_notifies() {
    let rules = NotificationRules::default();
    assert!(rules.admits(&cluster(), &warning("e", "web-0", "BackOff", 1, 0)));
}

#[test]
fn normal_events_and_unwatched_kinds_do_not() {
    let rules = NotificationRules::default();
    let mut normal = warning("e", "web-0", "BackOff", 1, 0);
    normal.event_type = EventType::Normal;
    assert!(!rules.admits(&cluster(), &normal));
    let configmap = event()
        .warning()
        .regarding("ConfigMap", Some("default"), "cfg", "cfg")
        .build();
    assert!(!rules.admits(&cluster(), &configmap));
}

#[test]
fn kinds_match_case_insensitively_and_can_be_narrowed() {
    let rules = NotificationRules {
        watched_kinds: vec![Arc::from("deployment")],
        ..NotificationRules::default()
    };
    let deployment = event()
        .warning()
        .regarding("Deployment", Some("default"), "api", "api")
        .build();
    assert!(rules.admits(&cluster(), &deployment));
    assert!(
        !rules.admits(&cluster(), &warning("e", "web-0", "BackOff", 1, 0)),
        "pods no longer"
    );
}

#[test]
fn an_ignored_reason_never_notifies() {
    let rules = NotificationRules {
        ignored_reasons: vec![Arc::from("BackOff")],
        ..NotificationRules::default()
    };
    assert!(!rules.admits(&cluster(), &warning("e", "web-0", "BackOff", 1, 0)));
    assert!(rules.admits(&cluster(), &warning("e", "web-0", "Failed", 1, 0)));
}

#[test]
fn the_event_filter_asks_for_the_watched_warnings_only() {
    let rules = NotificationRules::default();
    let filter = rules.event_filter(&cluster());
    assert!(filter.is_warnings_only());
    assert!(filter.matches(&warning("e", "web-0", "BackOff", 1, 0)));
    let configmap = event()
        .warning()
        .regarding("ConfigMap", Some("default"), "cfg", "cfg")
        .build();
    assert!(!filter.matches(&configmap));
}

#[test]
fn an_event_becomes_an_input_keyed_by_cluster_object_and_reason() {
    let rules = NotificationRules::default();
    let e = warning("e1", "web-0", "BackOff", 4, 0);
    let input = rules.input_for(&cluster(), &e, 3);
    assert_eq!(input.cluster, cluster());
    assert_eq!(input.level, NotificationLevel::Warning);
    assert_eq!(input.title, "Pod default/web-0: BackOff");
    assert_eq!(input.body, "BackOff on web-0");
    assert_eq!(input.target.as_ref(), Some(&e.regarding));
    assert_eq!(
        &*input.dedupe_key,
        &format!("event/{}/web-0/BackOff", cluster())
    );
    assert_eq!(input.occurrences, 3);
    let group = input.group.expect("grouped by reason");
    assert_eq!((&*group.reason, &*group.noun), ("BackOff", "pod"));

    // Another event about the same object and reason shares the key; another reason does not.
    let again = rules.input_for(&cluster(), &warning("e2", "web-0", "BackOff", 1, 9), 1);
    assert_eq!(again.dedupe_key, input.dedupe_key);
    let other = rules.input_for(&cluster(), &warning("e3", "web-0", "Failed", 1, 9), 1);
    assert_ne!(other.dedupe_key, input.dedupe_key);
}

fn deployment() -> oxikube_domain::event::Event {
    event()
        .warning()
        .regarding("Deployment", Some("default"), "api", "api")
        .build()
}

#[test]
fn a_cluster_can_watch_other_kinds_and_ignore_other_reasons() {
    let rules = NotificationRules {
        clusters: [(
            other(),
            ClusterRules {
                watched_kinds: Some(vec![Arc::from("Deployment")]),
                ignored_reasons: Some(vec![Arc::from("Failed")]),
                os_notifications: None,
            },
        )]
        .into(),
        ..NotificationRules::default()
    };
    let pod = warning("e", "web-0", "BackOff", 1, 0);
    // The cluster without an override follows the global lists.
    assert!(rules.admits(&cluster(), &pod));
    assert!(rules.admits(&cluster(), &deployment()));
    // The overridden one watches only Deployments, and its ignore-list replaces the global one.
    assert!(!rules.admits(&other(), &pod));
    assert!(rules.admits(&other(), &deployment()));
    assert!(!rules.admits(&other(), &warning("e", "web-0", "Failed", 1, 0)));
    assert_eq!(rules.watched_kinds_for(&other()).len(), 1);
    assert_eq!(
        rules.event_filter(&other()),
        NotificationRules {
            watched_kinds: vec![Arc::from("Deployment")],
            ..NotificationRules::default()
        }
        .event_filter(&cluster())
    );
    assert!(!rules.event_filter(&other()).matches(&pod));
}

#[test]
fn only_an_explicit_off_keeps_a_cluster_off_the_desktop() {
    let rules = NotificationRules {
        clusters: [(
            other(),
            ClusterRules {
                os_notifications: Some(false),
                ..ClusterRules::default()
            },
        )]
        .into(),
        ..NotificationRules::default()
    };
    assert!(rules.os_allowed(&cluster()));
    assert!(!rules.os_allowed(&other()));
}
