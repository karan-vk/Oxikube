//! [`EventsModel`]: the rows of the events view, kept in order as the service's changes arrive,
//! the selection, and the live tail's pause. No window in it: unit tests drive it with
//! `EventsChanged` values.
//!
//! A full list (the first, after a filter change, a relist, a resume) is built by
//! [`RowSet::build`], which the view runs on the background executor, and swapped in with
//! [`EventsModel::install`]. Between lists, [`EventsModel::apply`] takes one coalesced
//! `EventsChanged` (one per frame at most): each added, updated or removed event finds its place
//! with a binary search, so a delta costs `O(log n)` comparisons plus a move of the row vector's
//! tail (pointers: rows are boxed), never a re-sort (docs/PERFORMANCE.md rule 4). A burst of more
//! than [`REBUILD_MIN`] changes is applied in one pass instead: the changed rows are dropped, the
//! new ones appended and the vector sorted once, which the sort does in linear time (two sorted
//! runs to merge).
//!
//! While the tail is paused nothing moves: the changes are only counted ("12 new events") and
//! the list is read again when it resumes, so it resumes with every change in order.

mod row;
mod selection;
mod sort;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use oxikube_app::events::{EventKey, EventsChanged};
use oxikube_domain::event::Event;

pub use row::{EventColumn, EventRow, MESSAGE_CHARS, object_label, one_line, type_label};
pub use sort::EventSort;

/// A burst of more changes than this is applied in one pass (drop, append, one sort of two
/// sorted runs), which is cheaper than that many binary-searched inserts.
pub const REBUILD_MIN: usize = 64;

/// Rows in order with their index by key, built off the UI thread.
#[derive(Debug, Default)]
pub struct RowSet {
    // Boxed on purpose: a row is a few hundred bytes, and a new event on top of 10 000 rows moves
    // the whole vector; pointers make that move 30 times smaller.
    #[expect(
        clippy::vec_box,
        reason = "inserting at the top moves pointers, not rows"
    )]
    rows: Vec<Box<EventRow>>,
    by_key: HashMap<EventKey, (u64, Arc<Event>)>,
    next_id: u64,
}

impl RowSet {
    /// Rows for `events` (newest last seen first, as the service returns a query) in `sort`'s
    /// order. Arrival numbers start at `first_id`, the oldest event getting the smallest so ties
    /// keep the service's order.
    pub fn build(events: Vec<Arc<Event>>, sort: EventSort, first_id: u64) -> Self {
        let count = events.len() as u64;
        let mut by_key = HashMap::with_capacity(events.len());
        let mut rows: Vec<Box<EventRow>> = events
            .into_iter()
            .enumerate()
            .map(|(i, event)| {
                let id = first_id + count - 1 - i as u64;
                let key = EventKey::of(&event);
                by_key.insert(key.clone(), (id, event.clone()));
                Box::new(EventRow::new(id, key, event))
            })
            .collect();
        if !sort.is_service_order() {
            rows.sort_by(|a, b| sort.compare(a, b));
        }
        Self {
            rows,
            by_key,
            next_id: first_id + count,
        }
    }

    /// How many rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Whether the live tail runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tail {
    /// New events show as they arrive.
    #[default]
    Live,
    /// Changes are counted, not shown. `auto`: paused because the user scrolled away from the
    /// top (scrolling back resumes it); otherwise by the user (`events::TogglePause`).
    Paused {
        /// Paused by scrolling away from the top.
        auto: bool,
    },
}

/// The rows, the selection and the tail. See the [module docs](self).
#[derive(Debug, Default)]
pub struct EventsModel {
    set: RowSet,
    sort: EventSort,
    selected: Option<EventKey>,
    tail: Tail,
    /// Events that arrived while paused and are not shown.
    unseen: HashSet<EventKey>,
    /// Something changed while paused: resuming reads the list again.
    stale: bool,
}

impl EventsModel {
    /// An empty model in the default order.
    pub fn new() -> Self {
        Self::default()
    }

    /// The rows, in order.
    pub fn rows(&self) -> &[Box<EventRow>] {
        &self.set.rows
    }

    /// The rows, to fill their cell caches while drawing.
    pub fn rows_mut(&mut self) -> &mut [Box<EventRow>] {
        &mut self.set.rows
    }

    /// How many rows.
    pub fn len(&self) -> usize {
        self.set.rows.len()
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.set.rows.is_empty()
    }

    /// The events, in order.
    pub fn events(&self) -> impl Iterator<Item = &Arc<Event>> {
        self.set.rows.iter().map(|row| &row.event)
    }

    /// The order.
    pub fn sort(&self) -> EventSort {
        self.sort
    }

    /// The arrival number the next full list starts at.
    pub fn next_id(&self) -> u64 {
        self.set.next_id
    }

    /// Replaces the rows with a list built by [`RowSet::build`] (in this model's order). The
    /// selection stays on its event when the list still has it. Counts nothing as unseen.
    pub fn install(&mut self, set: RowSet) {
        self.set = set;
        if let Some(key) = &self.selected
            && !self.set.by_key.contains_key(key)
        {
            self.selected = None;
        }
        self.unseen.clear();
        self.stale = false;
    }

    /// Orders the rows by `sort` (a header click): one sort of the rows.
    pub fn set_sort(&mut self, sort: EventSort) {
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        self.set.rows.sort_by(|a, b| sort.compare(a, b));
    }

    /// Applies one coalesced change that is not a reset (a reset needs the list read again).
    /// Returns whether a row changed. While paused, nothing moves: the change is only counted.
    pub fn apply(&mut self, changed: &EventsChanged) -> bool {
        debug_assert!(!changed.reset, "a reset is a new list, not a change");
        if self.is_paused() {
            self.count_unseen(changed);
            return false;
        }
        let touched = changed.added.len() + changed.updated.len() + changed.removed.len();
        if touched == 0 {
            return false;
        }
        if touched > REBUILD_MIN {
            self.apply_by_sorting(changed);
        } else {
            for key in &changed.removed {
                self.remove(key);
            }
            for event in changed.updated.iter().chain(&changed.added) {
                self.upsert(event.clone());
            }
        }
        if let Some(key) = &self.selected
            && !self.set.by_key.contains_key(key)
        {
            self.selected = None;
        }
        true
    }

    fn count_unseen(&mut self, changed: &EventsChanged) {
        self.stale |= changed.reset || !changed.is_empty();
        for event in &changed.added {
            let key = EventKey::of(event);
            if !self.set.by_key.contains_key(&key) {
                self.unseen.insert(key);
            }
        }
        for key in &changed.removed {
            self.unseen.remove(key);
        }
    }

    /// The position of the row with `id` and `event` (the state it was placed with).
    fn position(&self, id: u64, event: &Event) -> Option<usize> {
        let sort = self.sort;
        self.set
            .rows
            .binary_search_by(|row| sort.order((row.id, &row.event), (id, event)))
            .ok()
    }

    fn remove(&mut self, key: &EventKey) {
        let Some((id, event)) = self.set.by_key.remove(key) else {
            return;
        };
        if let Some(at) = self.position(id, &event) {
            self.set.rows.remove(at);
        }
    }

    fn upsert(&mut self, event: Arc<Event>) {
        let key = EventKey::of(&event);
        let id = match self.set.by_key.get(&key) {
            Some((id, _)) => {
                let id = *id;
                self.remove(&key);
                id
            }
            None => {
                let id = self.set.next_id;
                self.set.next_id += 1;
                id
            }
        };
        let sort = self.sort;
        let at = self
            .set
            .rows
            .partition_point(|other| sort.order((other.id, &other.event), (id, &event)).is_lt());
        self.set
            .rows
            .insert(at, Box::new(EventRow::new(id, key.clone(), event.clone())));
        self.set.by_key.insert(key, (id, event));
    }

    fn apply_by_sorting(&mut self, changed: &EventsChanged) {
        let mut gone: HashSet<EventKey> = changed.removed.iter().cloned().collect();
        let mut fresh = Vec::with_capacity(changed.added.len() + changed.updated.len());
        for event in changed.updated.iter().chain(&changed.added) {
            let key = EventKey::of(event);
            let id = match self.set.by_key.get(&key) {
                Some((id, _)) => {
                    gone.insert(key.clone());
                    *id
                }
                None => {
                    let id = self.set.next_id;
                    self.set.next_id += 1;
                    id
                }
            };
            fresh.push(Box::new(EventRow::new(id, key, event.clone())));
        }
        self.set.rows.retain(|row| !gone.contains(&row.key));
        for key in &changed.removed {
            self.set.by_key.remove(key);
        }
        for row in &fresh {
            self.set
                .by_key
                .insert(row.key.clone(), (row.id, row.event.clone()));
        }
        self.set.rows.extend(fresh);
        let sort = self.sort;
        self.set.rows.sort_by(|a, b| sort.compare(a, b));
    }

    // --- The tail ------------------------------------------------------------------------

    /// The tail's state.
    pub fn tail(&self) -> Tail {
        self.tail
    }

    /// Whether the tail is paused.
    pub fn is_paused(&self) -> bool {
        matches!(self.tail, Tail::Paused { .. })
    }

    /// Pauses the tail (`auto`: because the user scrolled away from the top). A manual pause is
    /// not turned into an automatic one.
    pub fn pause(&mut self, auto: bool) {
        if !self.is_paused() {
            self.tail = Tail::Paused { auto };
        }
    }

    /// Resumes the tail. Returns whether the list must be read again (something changed while
    /// paused); the unseen count is cleared when it is.
    pub fn resume(&mut self) -> bool {
        self.tail = Tail::Live;
        self.unseen.clear();
        std::mem::take(&mut self.stale)
    }

    /// How many events arrived while paused and are not shown.
    pub fn unseen(&self) -> usize {
        self.unseen.len()
    }

    /// Marks the list as needing a fresh read on resume (a reset arrived while paused).
    pub fn mark_stale(&mut self) {
        self.stale = true;
    }
}

#[cfg(test)]
mod tests;
