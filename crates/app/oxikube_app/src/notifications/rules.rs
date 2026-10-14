//! [`NotificationRules`]: which events notify (kinds to watch, reasons to ignore), and the
//! dedupe, rate-limit and grouping windows. The defaults here are what the centre runs with until
//! the `events` settings (E14-S09) supply their own (`oxikube_events_ui::EventsSettings`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::event::{Event, EventType};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::notification::NotificationKind;
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

/// What one cluster does differently from the rest (`clusters.<id>.events`). A field left `None`
/// follows the rules' own value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClusterRules {
    /// The kinds whose `Warning` events notify for this cluster.
    pub watched_kinds: Option<Vec<Arc<str>>>,
    /// The reasons that never notify for this cluster.
    pub ignored_reasons: Option<Vec<Arc<str>>>,
    /// `Some(false)` keeps this cluster's notifications off the desktop (they are still listed
    /// and toasted in the app). `Some(true)` is the same as `None`: the desktop stays opt-in as
    /// a whole (`events.os_notifications`), a cluster cannot switch it on.
    pub os_notifications: Option<bool>,
}

/// What the centre notifies about and how it keeps a burst from becoming spam.
///
/// Matching an event is `O(kinds + ignored reasons)` (two short lists) plus one hash lookup for
/// a cluster's overrides, whatever the number of events held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRules {
    /// The kinds whose `Warning` events notify (`Pod`; compared case-insensitively).
    pub watched_kinds: Vec<Arc<str>>,
    /// Reasons that never notify (`BackOff`; compared exactly).
    pub ignored_reasons: Vec<Arc<str>>,
    /// The clusters whose kinds, ignored reasons or desktop switch differ from the above.
    pub clusters: HashMap<ClusterId, ClusterRules>,
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
            clusters: HashMap::new(),
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
    /// The kinds whose `Warning` events notify in `cluster`.
    pub fn watched_kinds_for(&self, cluster: &ClusterId) -> &[Arc<str>] {
        self.clusters
            .get(cluster)
            .and_then(|rules| rules.watched_kinds.as_deref())
            .unwrap_or(&self.watched_kinds)
    }

    /// The reasons that never notify in `cluster`.
    pub fn ignored_reasons_for(&self, cluster: &ClusterId) -> &[Arc<str>] {
        self.clusters
            .get(cluster)
            .and_then(|rules| rules.ignored_reasons.as_deref())
            .unwrap_or(&self.ignored_reasons)
    }

    /// Whether `cluster`'s notifications may reach the desktop (once the user opted in at all).
    pub fn os_allowed(&self, cluster: &ClusterId) -> bool {
        self.clusters
            .get(cluster)
            .and_then(|rules| rules.os_notifications)
            .unwrap_or(true)
    }

    /// Whether `event` of `cluster` notifies: a `Warning` about a kind watched there whose
    /// reason is not ignored there.
    pub fn admits(&self, cluster: &ClusterId, event: &Event) -> bool {
        event.event_type == EventType::Warning
            && self
                .watched_kinds_for(cluster)
                .iter()
                .any(|kind| kind.eq_ignore_ascii_case(&event.regarding.gvk.kind))
            && !self
                .ignored_reasons_for(cluster)
                .iter()
                .any(|reason| **reason == *event.reason)
    }

    /// The events filter `cluster`'s subscription uses: the warnings about the kinds watched
    /// there (the ignore-list is applied by [`admits`](Self::admits)).
    pub fn event_filter(&self, cluster: &ClusterId) -> EventFilter {
        EventFilter::warnings().kinds(self.watched_kinds_for(cluster).iter().cloned())
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
            kind: NotificationKind::ClusterEvent,
            cluster: Some(cluster.clone()),
            level: NotificationLevel::Warning,
            title: format!("{} {name}: {}", object.gvk.kind, event.reason),
            body: event.message.clone(),
            target: Some(object.clone()),
            link: None,
            dedupe_key: Arc::from(format!("event/{cluster}/{identity}/{}", event.reason)),
            group: Some(GroupKey {
                reason: event.reason.clone(),
                noun: Arc::from(object.gvk.kind.to_ascii_lowercase()),
            }),
            occurrences: occurrences.max(1),
        }
    }
}
