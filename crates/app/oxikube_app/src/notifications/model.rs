//! The centre's records: a [`Notification`] row, what a source hands in ([`NotificationInput`])
//! and the [`NotificationId`] that names a row.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_ports::NotificationLevel;
use serde::{Deserialize, Serialize};

/// The longest body kept on a row, in characters (an event message may be 4 KiB). Longer bodies
/// end in an ellipsis.
pub const MAX_BODY_CHARS: usize = 512;

/// Names one notification of the centre for as long as it is listed. Never reused within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NotificationId(pub u64);

impl std::fmt::Display for NotificationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// What groups a burst: notifications of one cluster with the same reason ("12 pods:
/// BackOff"). `noun` names the objects ("pod"); a group of mixed nouns says "objects".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupKey {
    /// The reason shared by the burst (`BackOff`).
    pub reason: Arc<str>,
    /// What one object is called, singular and lower case (`pod`).
    pub noun: Arc<str>,
}

/// One occurrence of something worth telling, as a source hands it to the centre
/// ([`NotificationService::submit`](super::NotificationService::submit)). Event sources build it
/// with [`NotificationRules::input_for`](super::NotificationRules::input_for); other sources
/// (E14-S07: port-forward drops, updates, agent permission) build their own.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationInput {
    /// The cluster it concerns: muting the cluster drops it.
    pub cluster: ClusterId,
    /// How urgent it is.
    pub level: NotificationLevel,
    /// The headline. Redacted by the centre.
    pub title: String,
    /// The detail line. Redacted and capped by the centre.
    pub body: String,
    /// The object a click opens, if any.
    pub target: Option<ResourceRef>,
    /// Inputs with the same key within the dedupe window update one row (its count goes up)
    /// instead of adding rows. Sources choose it: an event's is its cluster, object and reason.
    pub dedupe_key: Arc<str>,
    /// Inputs that may fold into one group row during a burst, if any.
    pub group: Option<GroupKey>,
    /// How many occurrences this input stands for (at least 1): a Kubernetes event's count can
    /// jump by more than one between two deliveries.
    pub occurrences: u32,
}

/// One row of the notification centre: a notification, or a group of them.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    /// The row's id.
    pub id: NotificationId,
    /// The cluster it concerns.
    pub cluster: ClusterId,
    /// How urgent it is.
    pub level: NotificationLevel,
    /// The headline (redacted).
    pub title: String,
    /// The latest detail (redacted, at most [`MAX_BODY_CHARS`]).
    pub body: String,
    /// The object a click opens: the latest one, for a group.
    pub target: Option<ResourceRef>,
    /// The key later occurrences are merged by.
    pub dedupe_key: Arc<str>,
    /// How many occurrences the row stands for.
    pub count: u32,
    /// How many distinct objects a group row covers; 1 for a plain row.
    pub objects: u32,
    /// Whether this row folds a burst of several objects ("12 pods: BackOff").
    pub grouped: bool,
    /// When the row was added.
    pub created: Timestamp,
    /// When the last occurrence arrived.
    pub updated: Timestamp,
    /// Whether the user has seen it (marked read, or opened it). A new occurrence makes it
    /// unread again.
    pub read: bool,
}

impl Notification {
    /// The same notification for the OS notifier (`NotifierPort`): its text, level, cluster, the
    /// object a click opens, and the dedupe key as the replacement tag, so a later occurrence
    /// replaces the one on the desktop. The port redacts and caps it again.
    pub fn to_os(&self) -> oxikube_ports::Notification {
        let mut os = oxikube_ports::Notification::new(&self.title, &self.body, self.level)
            .with_cluster(self.cluster.clone())
            .with_tag(self.dedupe_key.to_string());
        if let Some(target) = &self.target {
            os = os.with_action_target(target.clone());
        }
        os
    }
}

/// `text` redacted, on one line, at most `max` characters (the last one an ellipsis when cut).
pub(crate) fn clean(text: &str, max: usize) -> String {
    let redacted = oxikube_domain::redact::redact(text);
    let flat = redacted
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    cut
}
