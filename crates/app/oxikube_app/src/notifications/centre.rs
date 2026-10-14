//! [`Centre`]: the list of notifications, its dedupe and group rows, read state and bound.
//!
//! Rows are kept by id with an order index (newest last) and a key index, so adding, updating,
//! reading and evicting are `O(log n)` and a snapshot is one walk. Rows are `Arc`s: a snapshot
//! shares them, and an update copies only the row it changes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use jiff::Timestamp;

use super::limiter::{Limiter, Route, elapsed};
use super::model::{
    GroupKey, MAX_BODY_CHARS, Notification, NotificationId, NotificationInput, clean,
};
use super::rules::NotificationRules;

/// The longest title kept, in characters.
const MAX_TITLE_CHARS: usize = 160;

/// The most distinct objects a group row remembers; beyond it the count stops growing.
const MAX_GROUP_OBJECTS: usize = 4_096;

/// What [`Centre::apply`] did with an input.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Applied {
    /// The row it added or updated, as it is now.
    pub(crate) row: Arc<Notification>,
    /// Whether it should toast and reach the OS (new enough, and within the rate limit).
    pub(crate) raised: bool,
}

struct Row {
    note: Arc<Notification>,
    seq: u64,
    last_raised: Option<Timestamp>,
    group: Option<Group>,
}

struct Group {
    key: GroupKey,
    /// The objects folded in (their dedupe keys), up to [`MAX_GROUP_OBJECTS`].
    objects: HashSet<Arc<str>>,
    /// Whether the objects are of different kinds.
    mixed: bool,
}

/// The notification list. See the [module docs](self).
#[derive(Default)]
pub(crate) struct Centre {
    rows: HashMap<NotificationId, Row>,
    order: BTreeMap<u64, NotificationId>,
    by_key: HashMap<Arc<str>, NotificationId>,
    unread: usize,
    next_id: u64,
    next_seq: u64,
}

impl Centre {
    /// Adds `input` at `now`: to the row with its key while that row is within the dedupe window,
    /// else to the reason's group row during a burst, else to a new row. See
    /// [`Limiter`] for the routing and raising.
    pub(crate) fn apply(
        &mut self,
        input: NotificationInput,
        limiter: &mut Limiter,
        rules: &NotificationRules,
        now: Timestamp,
    ) -> Applied {
        let route = match (&input.group, self.live(&input.dedupe_key, rules, now)) {
            (Some(group), None) => limiter.route(rules, input.cluster.as_ref(), &group.reason, now),
            _ => Route::Own,
        };
        let key: Arc<str> = match (route, &input.group) {
            (Route::Group, Some(group)) => Arc::from(match &input.cluster {
                Some(cluster) => format!("group/{cluster}/{}", group.reason),
                None => format!("group/{}", group.reason),
            }),
            _ => input.dedupe_key.clone(),
        };
        let mut escalated = false;
        let id = match self.live(&key, rules, now) {
            Some(id) => {
                escalated = self.update(id, &input, now);
                id
            }
            None => self.insert(key, &input, route, now),
        };
        if route == Route::Group {
            self.fold(id, &input);
        }
        let row = self.rows.get_mut(&id).expect("the row was just written");
        let raised = limiter.may_raise(rules, row.last_raised, row.note.sticky(), escalated, now);
        if raised {
            row.last_raised = Some(now);
        }
        let row = row.note.clone();
        self.evict(rules.capacity);
        Applied { row, raised }
    }

    /// The rows, newest (by last occurrence) first.
    pub(crate) fn list(&self) -> Vec<Arc<Notification>> {
        self.order
            .values()
            .rev()
            .filter_map(|id| self.rows.get(id).map(|row| row.note.clone()))
            .collect()
    }

    /// The row `id`, if it is still listed.
    pub(crate) fn get(&self, id: NotificationId) -> Option<Arc<Notification>> {
        self.rows.get(&id).map(|row| row.note.clone())
    }

    /// How many rows are unread.
    pub(crate) fn unread(&self) -> usize {
        self.unread
    }

    /// How many rows are listed.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// Marks `id` read. Returns whether it was unread.
    pub(crate) fn mark_read(&mut self, id: NotificationId) -> bool {
        let Some(row) = self.rows.get_mut(&id) else {
            return false;
        };
        if row.note.read {
            return false;
        }
        Arc::make_mut(&mut row.note).read = true;
        self.unread -= 1;
        true
    }

    /// Marks the row with `key` read, if it is listed and unread (a request was answered).
    /// Returns whether it was unread.
    pub(crate) fn mark_read_by_key(&mut self, key: &str) -> bool {
        match self.by_key.get(key).copied() {
            Some(id) => self.mark_read(id),
            None => false,
        }
    }

    /// Marks every row read. Returns how many were unread.
    pub(crate) fn mark_all_read(&mut self) -> usize {
        let marked = self.unread;
        if marked > 0 {
            for row in self.rows.values_mut().filter(|row| !row.note.read) {
                Arc::make_mut(&mut row.note).read = true;
            }
            self.unread = 0;
        }
        marked
    }

    /// The row with `key` while its last occurrence is within the dedupe window.
    fn live(&self, key: &str, rules: &NotificationRules, now: Timestamp) -> Option<NotificationId> {
        let id = *self.by_key.get(key)?;
        let row = self.rows.get(&id)?;
        (!elapsed(row.note.updated, now, rules.dedupe_window)).then_some(id)
    }

    fn insert(
        &mut self,
        key: Arc<str>,
        input: &NotificationInput,
        route: Route,
        now: Timestamp,
    ) -> NotificationId {
        self.next_id += 1;
        let id = NotificationId(self.next_id);
        let group = match (route, &input.group) {
            (Route::Group, Some(group)) => Some(Group {
                key: group.clone(),
                objects: HashSet::new(),
                mixed: false,
            }),
            _ => None,
        };
        let note = Notification {
            id,
            kind: input.kind,
            cluster: input.cluster.clone(),
            level: input.level,
            title: clean(&input.title, MAX_TITLE_CHARS),
            body: clean(&input.body, MAX_BODY_CHARS),
            target: input.target.clone(),
            link: input.link.clone(),
            dedupe_key: key.clone(),
            count: input.occurrences.max(1),
            objects: 1,
            grouped: group.is_some(),
            created: now,
            updated: now,
            read: false,
        };
        let seq = self.bump();
        self.order.insert(seq, id);
        // A key whose row went out of the dedupe window now names the new row; the old row stays
        // listed under its old key, untouched.
        self.by_key.insert(key, id);
        self.rows.insert(
            id,
            Row {
                note: Arc::new(note),
                seq,
                last_raised: None,
                group,
            },
        );
        self.unread += 1;
        id
    }

    /// Folds `input` into row `id`; returns whether the row's level went up.
    fn update(&mut self, id: NotificationId, input: &NotificationInput, now: Timestamp) -> bool {
        let seq = self.bump();
        let Some(row) = self.rows.get_mut(&id) else {
            return false;
        };
        self.order.remove(&row.seq);
        self.order.insert(seq, id);
        row.seq = seq;
        let note = Arc::make_mut(&mut row.note);
        note.count = note.count.saturating_add(input.occurrences.max(1));
        note.updated = now;
        let before = note.level;
        note.level = before.max_with(input.level);
        let escalated = note.level != before;
        note.target = input.target.clone().or(note.target.take());
        note.link = input.link.clone().or(note.link.take());
        if row.group.is_none() {
            note.title = clean(&input.title, MAX_TITLE_CHARS);
            note.body = clean(&input.body, MAX_BODY_CHARS);
        }
        if note.read {
            note.read = false;
            self.unread += 1;
        }
        escalated
    }

    /// Adds `input`'s object to the group row `id` and words the row for the burst.
    fn fold(&mut self, id: NotificationId, input: &NotificationInput) {
        let Some(row) = self.rows.get_mut(&id) else {
            return;
        };
        let Some(group) = row.group.as_mut() else {
            return;
        };
        if group.objects.len() < MAX_GROUP_OBJECTS {
            group.objects.insert(input.dedupe_key.clone());
        }
        if let Some(theirs) = &input.group {
            group.mixed |= theirs.noun != group.key.noun;
        }
        let objects = group.objects.len().max(1);
        let noun = if group.mixed {
            "objects".to_owned()
        } else if objects == 1 {
            group.key.noun.to_string()
        } else {
            format!("{}s", group.key.noun)
        };
        let note = Arc::make_mut(&mut row.note);
        note.objects = u32::try_from(objects).unwrap_or(u32::MAX);
        note.title = clean(
            &format!("{objects} {noun}: {}", group.key.reason),
            MAX_TITLE_CHARS,
        );
        note.body = clean(
            &format!("Latest: {}. {}", input.title, input.body),
            MAX_BODY_CHARS,
        );
    }

    fn evict(&mut self, capacity: usize) {
        while self.rows.len() > capacity.max(1) {
            let Some((_, id)) = self.order.pop_first() else {
                return;
            };
            if let Some(row) = self.rows.remove(&id) {
                if !row.note.read {
                    self.unread -= 1;
                }
                if self.by_key.get(&row.note.dedupe_key) == Some(&id) {
                    self.by_key.remove(&row.note.dedupe_key);
                }
            }
        }
    }

    fn bump(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }
}

/// The more urgent of two levels.
trait LevelExt {
    fn max_with(self, other: Self) -> Self;
}

impl LevelExt for oxikube_ports::NotificationLevel {
    fn max_with(self, other: Self) -> Self {
        use oxikube_ports::NotificationLevel::{Error, Info, Success, Warning};
        let rank = |level| match level {
            Info => 0,
            Success => 1,
            Warning => 2,
            Error => 3,
        };
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }
}
