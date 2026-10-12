//! The Used by tab (E12-S18) of a ServiceAccount, ConfigMap, Secret, PersistentVolumeClaim or
//! PriorityClass: the workloads whose Pods name the object, each opening its own detail.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | [`UsedByPane`], the tab's state, and the view's side of it: starting and stopping, the poll for a changed index, the read that groups the Pods by workload, the answer |
//! | `body` | the drawing: the summary and notes above a virtualised list of workload rows |
//!
//! # What runs where
//!
//! Opening the tab takes a lease on the cluster's shared [`ReferenceFeed`]
//! ([`reference_feed`]): the first lease starts the Pods subscription, the last one released
//! (the tab is left, the detail closes) lets it go. While the tab is up a foreground task checks
//! the index's version (a number; five times a second while the Pods are being read, once a
//! second after) and, when it moved, starts one read on the Tokio bridge ([`used_by`]): the
//! Pods' owners, walked up to their workloads, each distinct owner read once and cached in the
//! feed. The answer is compared with the last and the view redraws only when it differs, so
//! Pod churn that changes no reference costs the UI nothing. A newer read replaces (cancels) an
//! older one.

mod body;

use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, ListAlignment, ListState, Task, px};
use oxikube_app::{ReferenceFeed, UsedBy, used_by};
use oxikube_domain::view::RefKind;
use oxikube_runtime::spawn_kube;

use super::view::DetailView;
use crate::used_by::reference_feed;

/// How often the tab looks for a changed index while the Pods are still being read.
const WARMING_POLL: Duration = Duration::from_millis(200);

/// How often it looks once they are known.
const STEADY_POLL: Duration = Duration::from_secs(1);

/// What the Used by tab keeps. Empty while another tab is shown: no lease, no answer.
pub(super) struct UsedByPane {
    /// The lease on the cluster's Pods subscription.
    feed: Option<Arc<ReferenceFeed>>,
    /// The latest answer; `None` until the first read lands.
    pub(super) answer: Option<UsedBy>,
    /// The index version the last read was started for.
    seen: u64,
    /// The workload rows' list state.
    pub(super) list: ListState,
    /// Looks for a changed index.
    poll: Option<Task<()>>,
    /// The read in flight; a newer one replaces it.
    read: Option<Task<()>>,
    /// How many reads were started (tests).
    #[cfg(test)]
    pub(super) reads: usize,
}

impl Default for UsedByPane {
    fn default() -> Self {
        Self {
            feed: None,
            answer: None,
            seen: u64::MAX,
            list: ListState::new(0, ListAlignment::Top, px(32.)),
            poll: None,
            read: None,
            #[cfg(test)]
            reads: 0,
        }
    }
}

impl UsedByPane {
    /// How many workload rows the answer lists.
    pub(super) fn rows(&self) -> usize {
        match &self.answer {
            Some(UsedBy::Known { workloads, .. }) => workloads.len(),
            _ => 0,
        }
    }

    /// Tells the list that the rows went from `old` to `new` in number.
    fn sync_list(&self, old: usize, new: usize) {
        if old != new {
            self.list.splice(0..old, new);
        }
        if new > 0 {
            self.list.remeasure_items(0..new);
        }
    }
}

impl DetailView {
    /// The kind of references the shown object is, `None` for any other kind (no tab).
    pub(super) fn ref_kind(&self) -> Option<RefKind> {
        RefKind::from_gvk(&self.target.gvk)
    }

    /// The tab was opened: takes the lease and starts looking.
    pub(super) fn used_by_tab_opened(&mut self, cx: &mut Context<Self>) {
        if self.ref_kind().is_none() || self.used_by.poll.is_some() {
            return;
        }
        self.used_by.seen = u64::MAX;
        self.used_by.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let wait = this.update(cx, |view, cx| view.poll_used_by(cx));
                match wait {
                    Ok(wait) => cx.background_executor().timer(wait).await,
                    Err(_) => break,
                }
            }
        }));
    }

    /// The tab was left or the view is going: lets the Pods subscription go (when no other view
    /// holds it) and forgets the answer.
    pub(super) fn used_by_tab_closed(&mut self) {
        self.used_by.poll = None;
        self.used_by.read = None;
        self.used_by.feed = None;
        self.used_by.answer = None;
        self.used_by.seen = u64::MAX;
        self.used_by.sync_list(self.used_by.list.item_count(), 0);
    }

    /// One look at the index: takes the lease on (or moves it to) the session's current store, and
    /// starts a read when the index changed. How long to wait before the next look.
    fn poll_used_by(&mut self, cx: &mut Context<Self>) -> Duration {
        let Some(kind) = self.ref_kind() else {
            return STEADY_POLL;
        };
        let env = self.columns_context();
        let Some(store) = env.store.clone() else {
            // Not connected (yet): the session's update brings the store, and the next look takes it.
            return WARMING_POLL;
        };
        if self
            .used_by
            .feed
            .as_ref()
            .is_none_or(|feed| !feed.reads(&store))
        {
            // The first lease, or a reconnect's new store: the old feed's Pods are frozen.
            self.used_by.feed = reference_feed(&env);
            self.used_by.seen = u64::MAX;
        }
        let Some(feed) = self.used_by.feed.clone() else {
            return WARMING_POLL;
        };
        let version = feed.index().version();
        if version != self.used_by.seen {
            self.used_by.seen = version;
            self.read_used_by(kind, &feed, cx);
        }
        match self.used_by.answer {
            None | Some(UsedBy::Warming) => WARMING_POLL,
            _ => STEADY_POLL,
        }
    }

    /// Starts the read that answers "what uses this?" off the UI thread.
    fn read_used_by(&mut self, kind: RefKind, feed: &Arc<ReferenceFeed>, cx: &mut Context<Self>) {
        let Some(reader) = self
            .deps
            .sessions
            .get(&self.target.cluster)
            .and_then(|session| session.resources())
        else {
            return;
        };
        let (index, owners) = (feed.index().clone(), feed.owners().clone());
        let cluster = self.target.cluster.clone();
        let namespace = self.target.namespace.clone();
        let name = self.target.name.clone();
        #[cfg(test)]
        {
            self.used_by.reads += 1;
        }
        let read = spawn_kube(cx, async move {
            used_by(
                &index,
                &owners,
                &*reader,
                &cluster,
                kind,
                namespace.as_deref(),
                &name,
            )
            .await
        });
        self.used_by.read = Some(cx.spawn(async move |this, cx| {
            let Ok(answer) = read.await else {
                return;
            };
            this.update(cx, |view, cx| view.set_used_by(answer, cx))
                .ok();
        }));
    }

    /// Takes a new answer; redraws only when it differs from the last.
    pub(super) fn set_used_by(&mut self, answer: UsedBy, cx: &mut Context<Self>) {
        if self.used_by.answer.as_ref() == Some(&answer) {
            return;
        }
        let old = self.used_by.rows();
        self.used_by.answer = Some(answer);
        self.used_by.sync_list(old, self.used_by.rows());
        cx.notify();
    }

    /// The latest answer (tests, and the palette's description of the tab).
    pub fn used_by_answer(&self) -> Option<&UsedBy> {
        self.used_by.answer.as_ref()
    }

    /// Whether the tab holds a lease on the cluster's Pods subscription.
    #[cfg(test)]
    pub(super) fn holds_reference_feed(&self) -> bool {
        self.used_by.feed.is_some()
    }
}
