//! [`ClusterBadges`]: the newest `Warning` event about each object of one cluster.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::event::Event;
use parking_lot::RwLock;

use crate::events::{EventKey, ObjectIdentity, ObjectTarget, seen_at};

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

/// Where a held warning lives: the map it is in, and the key it is under.
enum HeldIn {
    /// The event carried the object's uid.
    Uid(Arc<str>),
    /// The event gave no uid: it is held under the object's name.
    Named(ObjectIdentity),
}

#[derive(Default)]
struct Entries {
    /// The newest warning whose event carried the object's uid.
    by_uid: HashMap<Arc<str>, Held>,
    /// The newest warning whose event gave no uid, by the object's name.
    by_named: HashMap<ObjectIdentity, Held>,
    /// The key of each held event, to find its object when the event goes.
    by_key: HashMap<EventKey, HeldIn>,
}

impl Entries {
    /// Holds `event` for its object unless a newer warning is held. `false`: nothing changed.
    fn upsert(&mut self, event: &Arc<Event>) -> bool {
        let seen = seen_at(event);
        if seen == Timestamp::MIN {
            return false;
        }
        let key = EventKey::of(event);
        match &event.regarding_uid {
            Some(uid) => {
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
                self.by_key.insert(key.clone(), HeldIn::Uid(uid.clone()));
                self.by_uid.insert(
                    uid.clone(),
                    Held {
                        key,
                        event: event.clone(),
                        seen,
                    },
                );
            }
            None => {
                let identity = ObjectIdentity::of_event(event);
                if let Some(held) = self.by_named.get(&identity) {
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
                self.by_key
                    .insert(key.clone(), HeldIn::Named(identity.clone()));
                self.by_named.insert(
                    identity,
                    Held {
                        key,
                        event: event.clone(),
                        seen,
                    },
                );
            }
        }
        true
    }

    /// Forgets the held event with `key`; the object it was about, so that another warning
    /// about it can take its place.
    fn remove_key(&mut self, key: &EventKey) -> Option<ObjectTarget> {
        let where_ = self.by_key.remove(key)?;
        match where_ {
            HeldIn::Uid(uid) => {
                let held = self.by_uid.remove(&uid)?;
                Some(ObjectTarget::named(
                    Some(uid),
                    ObjectIdentity::of_event(&held.event),
                ))
            }
            HeldIn::Named(identity) => {
                self.by_named.remove(&identity)?;
                Some(ObjectTarget::named(None, identity))
            }
        }
    }

    fn clear(&mut self) {
        self.by_uid.clear();
        self.by_named.clear();
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

    /// [`get`](Self::get) for a row: by the object's uid, and for a cluster-scoped object also
    /// by `cluster_scoped_name` (the kubelet records a Node's events with the node's name as
    /// the uid). The events that gave no uid are found by the object's name, so a row is
    /// badged by them too. `O(1)`: at most three hash lookups.
    pub fn get_for(
        &self,
        target: &ObjectTarget,
        cluster_scoped_name: Option<&str>,
        now: Timestamp,
    ) -> Option<WarningBadge> {
        let cutoff = self.cutoff(now);
        let entries = self.entries.read();
        let mut newest: Option<&Held> = None;
        for held in [
            target.uid_value().and_then(|uid| entries.by_uid.get(uid)),
            cluster_scoped_name.and_then(|name| entries.by_uid.get(name)),
            target
                .identity()
                .and_then(|identity| entries.by_named.get(identity)),
        ] {
            if let Some(held) = held
                && newest.is_none_or(|best| held.seen > best.seen)
            {
                newest = Some(held);
            }
        }
        let held = newest?;
        (held.seen >= cutoff).then(|| WarningBadge {
            event: held.event.clone(),
        })
    }

    /// How many held warnings aged past the window between `from` and `to`: badged at `from`,
    /// not at `to`. A view that checked at `from` redraws when this is not zero. Counting the
    /// crossing itself (not how many entries are past the window) cannot miss an expiry that
    /// coincides with a replacement or an eviction. Reads every entry (the warned objects), so
    /// ask about once a second, not per row.
    pub fn expired_between(&self, from: Timestamp, to: Timestamp) -> usize {
        let (from, to) = (self.cutoff(from), self.cutoff(to));
        let entries = self.entries.read();
        let crossing = |held: &Held| held.seen >= from && held.seen < to;
        entries
            .by_uid
            .values()
            .filter(|held| crossing(held))
            .count()
            + entries
                .by_named
                .values()
                .filter(|held| crossing(held))
                .count()
    }

    /// How many held warnings there are (an object can hold two: one whose event carried its
    /// uid and one whose event did not).
    pub fn len(&self) -> usize {
        let entries = self.entries.read();
        entries.by_uid.len() + entries.by_named.len()
    }

    /// Whether no warning is held.
    pub fn is_empty(&self) -> bool {
        let entries = self.entries.read();
        entries.by_uid.is_empty() && entries.by_named.is_empty()
    }

    /// Replaces everything with `events` (newest first; the first per object wins).
    pub(crate) fn replace(&self, events: &[Arc<Event>]) {
        let mut entries = self.entries.write();
        entries.clear();
        for event in events {
            let held = match &event.regarding_uid {
                Some(uid) => !entries.by_uid.contains_key(uid),
                None => {
                    let identity = ObjectIdentity::of_event(event);
                    !entries.by_named.contains_key(&identity)
                }
            };
            if held {
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
    ) -> (bool, Vec<ObjectTarget>) {
        let mut entries = self.entries.write();
        let mut changed = false;
        let mut lost = Vec::new();
        for key in removed {
            if let Some(target) = entries.remove_key(key) {
                lost.push(target);
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
