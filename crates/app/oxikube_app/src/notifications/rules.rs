//! [`NotificationRules`]: which events notify (kinds to watch, reasons to ignore), and the
//! dedupe, rate-limit and grouping windows. The defaults here are what the centre runs with until
//! the `events` settings (E14-S09) supply their own.

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::event::{Event, EventType};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::NotificationLevel;

use super::model::{GroupKey, NotificationInput};
use crate::events::EventFilter;

/// The kinds whose `Warning` events notify by default: the workloads, their pods, nodes and
/// claims.
pub const DEFAULT_WATCHED_KINDS: [&str; 9] = [
    "Pod",
    "Deployment",
    "StatefulSet",
    "DaemonSet",
    "ReplicaSet",
    "Job",
    "CronJob",
    "Node",
    "PersistentVolumeClaim",
];

/// What the centre notifies about and how it keeps a burst from becoming spam.
///
/// Matching an event is `O(kinds + ignored reasons)`, two short lists, whatever the number of
/// events held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRules {
    /// The kinds whose `Warning` events notify (`Pod`; compared case-insensitively).
    pub watched_kinds: Vec<Arc<str>>,
    /// Reasons that never notify (`BackOff`; compared exactly).
    pub ignored_reasons: Vec<Arc<str>>,
    /// A new occurrence with the same dedupe key within this window updates the existing row
    /// (count up) instead of adding one.
    pub dedupe_window: Duration,
    /// One row toasts (and reaches the OS) at most once per this interval; occurrences in
    /// between only update the row.
    pub object_interval: Duration,
    /// The window a burst is counted in, per cluster and reason.
    pub group_window: Duration,
    /// Rows of one reason in one cluster within [`group_window`](Self::group_window) after which
    /// further objects fold into one group row ("12 pods: BackOff").
    pub group_threshold: usize,
    /// At most this many notifications toast (and reach the OS) per
    /// [`raise_window`](Self::raise_window), whatever their reasons; the others are only listed.
    pub max_raised: usize,
    /// The window [`max_raised`](Self::max_raised) counts in.
    pub raise_window: Duration,
    /// The most rows the centre keeps; the oldest go first.
    pub capacity: usize,
}

impl Default for NotificationRules {
    fn default() -> Self {
        Self {
            watched_kinds: DEFAULT_WATCHED_KINDS
                .iter()
                .map(|k| Arc::from(*k))
                .collect(),
            ignored_reasons: Vec::new(),
            dedupe_window: Duration::from_secs(10 * 60),
            object_interval: Duration::from_secs(5 * 60),
            group_window: Duration::from_secs(60),
            group_threshold: 3,
            max_raised: 5,
            raise_window: Duration::from_secs(30),
            capacity: 500,
        }
    }
}

impl NotificationRules {
    /// Whether `event` notifies: a `Warning` about a watched kind whose reason is not ignored.
    pub fn admits(&self, event: &Event) -> bool {
        event.event_type == EventType::Warning
            && self
                .watched_kinds
                .iter()
                .any(|kind| kind.eq_ignore_ascii_case(&event.regarding.gvk.kind))
            && !self
                .ignored_reasons
                .iter()
                .any(|reason| **reason == *event.reason)
    }

    /// The events filter a cluster's subscription uses: the warnings about the watched kinds
    /// (the ignore-list is applied by [`admits`](Self::admits)).
    pub fn event_filter(&self) -> EventFilter {
        EventFilter::warnings().kinds(self.watched_kinds.iter().cloned())
    }

    /// The input for one occurrence (or `occurrences` of them) of `event` in `cluster`: titled
    /// by its object and reason, with its message as the body, opening its object on a click,
    /// deduplicated per (cluster, object, reason) and grouped by reason.
    pub fn input_for(
        &self,
        cluster: &ClusterId,
        event: &Event,
        occurrences: u32,
    ) -> NotificationInput {
        let object = &event.regarding;
        let name = match &object.namespace {
            Some(ns) => format!("{ns}/{}", object.name),
            None => object.name.to_string(),
        };
        let identity = match &event.regarding_uid {
            Some(uid) => uid.to_string(),
            None => format!("{}/{name}", object.gvk.kind),
        };
        NotificationInput {
            cluster: cluster.clone(),
            level: NotificationLevel::Warning,
            title: format!("{} {name}: {}", object.gvk.kind, event.reason),
            body: event.message.clone(),
            target: Some(object.clone()),
            dedupe_key: Arc::from(format!("event/{cluster}/{identity}/{}", event.reason)),
            group: Some(GroupKey {
                reason: event.reason.clone(),
                noun: Arc::from(object.gvk.kind.to_ascii_lowercase()),
            }),
            occurrences: occurrences.max(1),
        }
    }
}
