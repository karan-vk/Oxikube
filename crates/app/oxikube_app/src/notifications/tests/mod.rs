//! Tests of the notification centre: the rules, the list (dedupe, grouping, rate limit, read
//! state, bound) on a manual clock, the tracker that keeps the backlog quiet, the service against
//! `FakeEventsPort` and `FakeStatePort` (mute persisted and restored), and the bus commands.

mod centre;
mod commands;
mod limiter;
mod rules;
mod service;
mod sources;
mod tracker;

use std::sync::Arc;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::event::Event;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_testkit::event;

use oxikube_domain::notification::NotificationKind;

use super::{GroupKey, NotificationInput};
use oxikube_ports::NotificationLevel;

/// The cluster of the testkit's events.
pub(super) fn cluster() -> ClusterId {
    ClusterId::new("kubeconfig", &ContextName::new("kind-oxikube"))
}

/// Another cluster.
pub(super) fn other() -> ClusterId {
    ClusterId::new("kubeconfig", &ContextName::new("prod"))
}

pub(super) fn t0() -> Timestamp {
    "2026-01-01T00:00:00Z".parse().expect("a timestamp")
}

/// `t0` plus `secs`.
pub(super) fn at(secs: i64) -> Timestamp {
    t0() + SignedDuration::from_secs(secs)
}

pub(super) fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// A `Warning` event `uid` with `reason` about pod `name` (uid `name`) in `default`, counted
/// `count` times, last seen `secs` after [`t0`].
pub(super) fn warning(uid: &str, name: &str, reason: &str, count: u32, secs: i64) -> Event {
    let mut e = event()
        .uid(uid)
        .warning()
        .reason(reason)
        .message(&format!("{reason} on {name}"))
        .regarding("Pod", Some("default"), name, name)
        .count(count)
        .build();
    e.last_seen = Some(at(secs));
    e.first_seen = Some(at(secs));
    e
}

/// A plain input (no group) with `key` in `cluster`.
pub(super) fn input(cluster: &ClusterId, key: &str) -> NotificationInput {
    NotificationInput {
        kind: NotificationKind::ClusterEvent,
        cluster: Some(cluster.clone()),
        level: NotificationLevel::Warning,
        title: format!("title {key}"),
        body: format!("body {key}"),
        target: None,
        link: None,
        dedupe_key: Arc::from(key),
        group: None,
        occurrences: 1,
    }
}

/// An input about pod `object` with `reason`, grouped by reason.
pub(super) fn grouped(cluster: &ClusterId, object: &str, reason: &str) -> NotificationInput {
    NotificationInput {
        group: Some(GroupKey {
            reason: Arc::from(reason),
            noun: Arc::from("pod"),
        }),
        title: format!("Pod default/{object}: {reason}"),
        ..input(cluster, &format!("event/{cluster}/{object}/{reason}"))
    }
}
