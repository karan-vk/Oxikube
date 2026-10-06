//! [`Subscription`]: the stream a view reads. Its rows live in a [`SubShared`] mailbox that the
//! feeds' drivers fill (see `mailbox`).

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;
use oxikube_domain::session::WatchScope;

use super::delta::{FeedState, StoreDelta};
use super::entry::{FeedEntry, SubId};
use super::mailbox::{SubShared, seed};
use super::object::FeedScope;
use super::policy::FeedKind;
use super::query::{SortKey, StoreFilter, StoreQuery};
use super::service::StoreInner;
use super::spawn::TaskGuard;
use crate::session::namespaces::ScopeDelta;

/// A live view of one [`StoreQuery`]: a [`Stream`] of [`StoreDelta`]s.
///
/// The first item is a snapshot (possibly empty and `Warming`); later items are coalesced
/// batches of [`RowOp`](super::RowOp)s, or a snapshot after a relist or a filter, sort or scope
/// change. Every method returns at once: filling or re-sorting the rows runs on the store's
/// spawner, and the stream yields the snapshot when it is done. The stream never ends on its own. Dropping the subscription releases its feeds: the last
/// subscriber of a feed starts the store's grace timer, after which the feed is aborted.
pub struct Subscription {
    store: Arc<StoreInner>,
    shared: Arc<SubShared>,
    id: SubId,
    query: StoreQuery,
    feeds: BTreeMap<FeedScope, Arc<FeedEntry>>,
    kind: FeedKind,
    /// The pending seeding task (abort-on-drop); a newer one replaces it.
    seeding: Option<TaskGuard>,
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("id", &self.id)
            .field("query", &self.query)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl Subscription {
    pub(crate) fn open(store: Arc<StoreInner>, query: StoreQuery) -> Self {
        let shared = Arc::new(SubShared::new(&query));
        let id = store.next_id();
        let kind = store.plan(&query.gvk).kind;
        let mut sub = Self {
            store,
            shared,
            id,
            query,
            feeds: BTreeMap::new(),
            kind,
            seeding: None,
        };
        for part in sub.query.parts() {
            sub.attach(part, &[]);
        }
        sub.spawn_seed();
        sub
    }

    fn attach(&mut self, part: FeedScope, seed_from: &[Arc<FeedEntry>]) {
        let entry = self
            .store
            .attach(&self.query.gvk, &part, self.id, &self.shared, seed_from);
        self.kind = entry.kind();
        self.feeds.insert(part, entry);
    }

    /// When a part is unseeded, starts a seeding task on the store's spawner (replacing, and so
    /// aborting, any earlier one). The task gets every part, since a rebuild it supersedes
    /// leaves every part for it.
    fn spawn_seed(&mut self) {
        if !self.shared.needs_seed() {
            return;
        }
        let parts: Vec<_> = self
            .feeds
            .iter()
            .map(|(part, entry)| (part.clone(), Arc::downgrade(entry)))
            .collect();
        self.seeding = Some(self.store.spawn(seed(Arc::downgrade(&self.shared), parts)));
    }

    /// The query as it stands now (after any filter, sort or scope change).
    pub fn query(&self) -> &StoreQuery {
        &self.query
    }

    /// Which feed serves this subscription (after any budget degrade).
    pub fn feed_kind(&self) -> FeedKind {
        self.feeds.values().next().map_or(self.kind, |e| e.kind())
    }

    /// The combined state of the subscription's feeds now.
    pub fn state(&self) -> FeedState {
        self.shared.state()
    }

    /// Replaces the in-app filter; the next item is a snapshot. No feed restarts.
    pub fn set_filter(&mut self, filter: StoreFilter) {
        if self.query.filter != filter {
            self.query.filter = filter;
            self.reseed();
        }
    }

    /// Replaces the sort order; the next item is a snapshot. No feed restarts.
    pub fn set_sort(&mut self, sort: SortKey) {
        if self.query.sort != sort {
            self.query.sort = sort;
            self.reseed();
        }
    }

    fn reseed(&mut self) {
        self.shared
            .reset(self.query.filter.clone(), self.query.sort.clone());
        self.spawn_seed();
    }

    /// Moves the subscription to a new scope (the session's namespace selection changed).
    ///
    /// Namespaces that stay selected keep their feed and rows ([`ScopeDelta`]); feeds no longer
    /// needed are released first, so the watch budget can count or evict them, and new ones
    /// then attach to a shared feed, which starts if needed and is seeded from the feeds being
    /// left (so narrowing from all namespaces to one shows that namespace's rows before the new
    /// feed lists, then its relist reconciles them). The subscription, its filter and sort
    /// survive, and the next item is a snapshot.
    pub fn rescope(&mut self, scope: WatchScope) {
        let delta = ScopeDelta::between(&self.query.scope, &scope);
        if delta.is_empty() && self.query.scope == scope {
            return;
        }
        let old: Vec<FeedScope> = self.feeds.keys().cloned().collect();
        self.query.scope = scope;
        let new = self.query.parts();
        let leaving: Vec<Arc<FeedEntry>> = old
            .iter()
            .filter(|p| !new.contains(p))
            .filter_map(|p| self.feeds.get(p).cloned())
            .collect();
        // Drop the leaving parts' rows (a cluster-wide part covers every namespace) and release
        // their feeds before the new ones ask the budget: a swap that fits must not be refused
        // because the feeds it replaces still count. Their entries stay alive to seed the new
        // feeds even if the release tears them down.
        for entry in &leaving {
            self.feeds.remove(&entry.key.scope);
            self.shared.detach_part(&entry.key.scope);
            self.store.detach(entry, self.id);
        }
        for part in new {
            if !self.feeds.contains_key(&part) {
                self.attach(part, &leaving);
            }
        }
        self.spawn_seed();
    }
}

impl Stream for Subscription {
    type Item = StoreDelta;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<StoreDelta>> {
        self.shared.poll(cx)
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        for entry in std::mem::take(&mut self.feeds).into_values() {
            self.store.detach(&entry, self.id);
        }
    }
}
