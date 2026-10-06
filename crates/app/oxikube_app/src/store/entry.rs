//! [`FeedEntry`]: one cache entry (one feed) of a store: its objects, state, Table columns,
//! subscribers, and the abort-on-drop guards of its driver and grace timer.
//!
//! Lock order across the store: the store's entry map, then an entry, then a subscriber. The
//! driver takes an entry then its subscribers; nothing takes them the other way round.

use std::sync::Arc;

use jiff::Timestamp;
use parking_lot::Mutex;

use super::budget::FeedRequest;
use super::cache::ObjectCache;
use super::delta::FeedState;
use super::feed::{FeedBatch, TableColumns};
use super::object::FeedKey;
use super::spawn::TaskGuard;
use super::subscription::SubShared;

/// A subscriber's id within one store.
pub(crate) type SubId = u64;

/// One feed's cache entry.
pub(crate) struct FeedEntry {
    pub key: FeedKey,
    /// What was asked of the budget (with the kind it granted).
    pub request: FeedRequest,
    /// Whether the budget admitted it (only admitted feeds run and are released).
    pub admitted: bool,
    pub state: Mutex<EntryState>,
}

/// The mutable part of a [`FeedEntry`].
pub(crate) struct EntryState {
    pub cache: ObjectCache,
    pub feed_state: FeedState,
    pub columns: Option<TableColumns>,
    /// The live subscribers: the entry's reference count.
    pub subscribers: Vec<(SubId, Arc<SubShared>)>,
    /// The feed task; `None` before it starts and after it stops.
    pub driver: Option<TaskGuard>,
    /// Whether the driver is still running (it clears this when it gives up).
    pub running: bool,
    /// The grace timer started when the last subscriber left.
    pub grace: Option<TaskGuard>,
    /// Bumped whenever the entry goes idle, so a stale grace timer does nothing.
    pub generation: u64,
    pub idle_since: Option<Timestamp>,
}

impl FeedEntry {
    pub fn new(key: FeedKey, request: FeedRequest, admitted: bool, state: FeedState) -> Self {
        Self {
            key,
            request,
            admitted,
            state: Mutex::new(EntryState {
                cache: ObjectCache::default(),
                feed_state: state,
                columns: None,
                subscribers: Vec::new(),
                driver: None,
                running: false,
                grace: None,
                generation: 0,
                idle_since: None,
            }),
        }
    }

    /// Applies one feed batch and hands the net change to every subscriber.
    pub fn apply(&self, batch: FeedBatch) {
        let mut st = self.state.lock();
        if let Some(columns) = &batch.columns
            && st.columns.as_ref() != Some(columns)
        {
            for (_, sub) in &st.subscribers {
                sub.set_columns(columns);
            }
            st.columns = Some(columns.clone());
        }
        let change = st.cache.apply(batch);
        let ready = change.restarted || matches!(st.feed_state, FeedState::Retrying { .. });
        if !change.is_empty() {
            for (_, sub) in &st.subscribers {
                sub.apply_change(&self.key.scope, &change);
            }
        }
        if ready && st.feed_state != FeedState::Ready {
            Self::publish(&mut st, &self.key, FeedState::Ready);
        }
    }

    /// Moves the feed to `state` and tells the subscribers.
    pub fn set_state(&self, state: FeedState) {
        let mut st = self.state.lock();
        if st.feed_state != state {
            Self::publish(&mut st, &self.key, state);
        }
    }

    /// The driver gave up (terminal error): a later subscriber restarts it.
    pub fn stopped(&self) {
        self.state.lock().running = false;
    }

    /// Sets `state` (the caller holds the entry lock) and tells every subscriber.
    pub fn publish(st: &mut EntryState, key: &FeedKey, state: FeedState) {
        tracing::debug!(feed = %key, state = state.label(), "resource store feed state");
        for (_, sub) in &st.subscribers {
            sub.set_part_state(&key.scope, &state);
        }
        st.feed_state = state;
    }
}
