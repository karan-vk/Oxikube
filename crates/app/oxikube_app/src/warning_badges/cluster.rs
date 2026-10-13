//! [`ClusterBadges`]: the newest `Warning` event about each object of one cluster.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::event::Event;
use parking_lot::RwLock;

use crate::events::EventKey;

/// When `event` was last seen: `last_seen`, else `first_seen`, else the oldest time.
pub(crate) fn seen_at(event: &Event) -> Timestamp {
    event
        .last_seen
        .or(event.first_seen)
        .unwrap_or(Timestamp::MIN)
}

/// The warning behind a badge: the newest `Warning` event about the object.
#[derive(Debug, Clone, PartialEq)]
pub struct WarningBadge {
    /// The event (cheap to clone).
    pub event: Arc<Event>,
}

impl WarningBadge {
    /// The event's reason (`BackOff`).
    pub fn reason(&self) -> &str {
        &self.event.reason
    }

    /// The event's message.
    pub fn message(&self) -> &str {
        &self.event.message
    }

    /// How many times it happened, at least 1.
    pub fn count(&self) -> u32 {
        self.event.count.max(1)
    }

    /// When it was last seen.
    pub fn last_seen(&self) -> Timestamp {
        seen_at(&self.event)
    }
}

struct Held {
    key: EventKey,
    event: Arc<Event>,
    seen: Timestamp,
}

#[derive(Default)]
struct Entries {
    by_uid: HashMap<Arc<str>, Held>,
    /// The key of each held event, to find its object when the event goes.
    by_key: HashMap<EventKey, Arc<str>>,
}

impl Entries {
    /// Holds `event` for its object unless a newer warning is held. `false`: nothing changed.
    fn upsert(&mut self, event: &Arc<Event>) -> bool {
        let Some(uid) = &event.regarding_uid else {
            return false;
        };
        let seen = seen_at(event);
        if seen == Timestamp::MIN {
            return false;
        }
        let key = EventKey::of(event);
        if let Some(held) = self.by_uid.get(uid) {
            if held.key != key && held.seen > seen {
                return false;
            }
            if held.key == key && Arc::ptr_eq(&held.event, event) {
                return false;
            }
            if held.key != key {
                self.by_key.remove(&held.key);
            }
        }
        self.by_key.insert(key.clone(), uid.clone());
        self.by_uid.insert(
            uid.clone(),
            Held {
                key,
                event: event.clone(),
                seen,
            },
        );
        true
    }

    /// Forgets the held event with `key`; its object, when one was held.
    fn remove_key(&mut self, key: &EventKey) -> Option<Arc<str>> {
        let uid = self.by_key.remove(key)?;
        self.by_uid.remove(&uid);
        Some(uid)
    }

    fn clear(&mut self) {
        self.by_uid.clear();
        self.by_key.clear();
    }
}

/// One cluster's badges. Cheap to clone; reads take a shared lock and look up one hash entry.
#[derive(Clone)]
pub struct ClusterBadges {
    entries: Arc<RwLock<Entries>>,
    window_secs: Arc<AtomicU64>,
}

impl ClusterBadges {
    pub(crate) fn new(window_secs: Arc<AtomicU64>) -> Self {
        Self {
            entries: Arc::default(),
            window_secs,
        }
    }

    fn cutoff(&self, now: Timestamp) -> Timestamp {
        let window = Duration::from_secs(self.window_secs.load(Ordering::Relaxed));
        now - SignedDuration::try_from(window).unwrap_or(SignedDuration::MAX)
    }

    /// The warning that gives the object with `uid` a badge at `now`: its newest `Warning`
    /// event, when last seen within the window. `O(1)`.
    pub fn get(&self, uid: &str, now: Timestamp) -> Option<WarningBadge> {
        let cutoff = self.cutoff(now);
        let entries = self.entries.read();
        let held = entries.by_uid.get(uid)?;
        (held.seen >= cutoff).then(|| WarningBadge {
            event: held.event.clone(),
        })
    }

    /// [`get`](Self::get) for a row: by `uid`, and for a cluster-scoped object also by `name`
    /// (the kubelet records a Node's events with the node's name as the uid).
    pub fn get_for(
        &self,
        uid: Option<&str>,
        cluster_scoped_name: Option<&str>,
        now: Timestamp,
    ) -> Option<WarningBadge> {
        uid.and_then(|uid| self.get(uid, now))
            .or_else(|| cluster_scoped_name.and_then(|name| self.get(name, now)))
    }

    /// How many held warnings are past the window at `now`. A view that last drew at `now0`
    /// redraws when this differs from what it drew with: a badge expired. Reads every entry
    /// (the warned objects), so ask about once a second, not per row.
    pub fn expired(&self, now: Timestamp) -> usize {
        let cutoff = self.cutoff(now);
        self.entries
            .read()
            .by_uid
            .values()
            .filter(|held| held.seen < cutoff)
            .count()
    }

    /// How many objects have a held warning (some may be past the window).
    pub fn len(&self) -> usize {
        self.entries.read().by_uid.len()
    }

    /// Whether no warning is held.
    pub fn is_empty(&self) -> bool {
        self.entries.read().by_uid.is_empty()
    }

    /// Replaces everything with `events` (newest first; the first per object wins).
    pub(crate) fn replace(&self, events: &[Arc<Event>]) {
        let mut entries = self.entries.write();
        entries.clear();
        for event in events {
            if event
                .regarding_uid
                .as_ref()
                .is_some_and(|uid| !entries.by_uid.contains_key(uid))
            {
                entries.upsert(event);
            }
        }
    }

    /// Applies one batch: the `removed` keys first (returning the objects that lost the event
    /// they showed), then `events` (oldest first). `true`: something changed.
    pub(crate) fn apply(
        &self,
        removed: &[EventKey],
        events: impl IntoIterator<Item = Arc<Event>>,
    ) -> (bool, Vec<Arc<str>>) {
        let mut entries = self.entries.write();
        let mut changed = false;
        let mut lost = Vec::new();
        for key in removed {
            if let Some(uid) = entries.remove_key(key) {
                lost.push(uid);
                changed = true;
            }
        }
        for event in events {
            changed |= entries.upsert(&event);
        }
        (changed, lost)
    }

    /// Holds `event` if it is newer than what the object has (a replacement found after the
    /// event it showed went). `true`: something changed.
    pub(crate) fn offer(&self, event: &Arc<Event>) -> bool {
        self.entries.write().upsert(event)
    }
}
