//! [`ReleaseWatch`]: one `()` whenever the cluster's release records or the view's scope change.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;
use futures::stream::BoxStream;
use oxikube_domain::ids::{ClusterId, Gvk, Scope};
use oxikube_domain::session::{NamespaceSelection, WatchScope};

use super::source::ReleaseSource;
use crate::session::{SessionChange, SessionUpdate};
use crate::store::{LabelSelector, ResourceStore, RowChange, StoreQuery, Subscription};

/// The storage drivers' kinds (`secrets` by default, `configmaps` with `HELM_DRIVER=configmap`).
fn kinds() -> [Gvk; 2] {
    [
        Gvk::new("", "v1", "Secret"),
        Gvk::new("", "v1", "ConfigMap"),
    ]
}

/// Live updates of a cluster's releases. A stream of `()`: each item says "list again".
///
/// It holds the store's feeds for Secrets and ConfigMaps labelled `owner=helm` (metadata only,
/// shared with any other view that watches them, released on drop) and follows the session:
/// a namespace change moves the feeds to the new scope and a connection change asks for a new
/// list. A connection change also looks the cluster's store up again: a reconnect builds a new
/// store (the old one is forgotten) and a disconnect leaves none, so the feeds move to the
/// current store, or wait without one until the cluster connects. A watch opened before the
/// cluster connected therefore starts working once it does. The first message of each feed (its snapshot) is not a change and is swallowed. There
/// is no timer: with no change the stream stays pending.
///
/// Items are not rate limited; the view debounces them.
pub struct ReleaseWatch {
    cluster: ClusterId,
    source: Arc<dyn ReleaseSource>,
    selection: NamespaceSelection,
    /// The store the feeds are on, `None` while the cluster is not connected.
    store: Option<ResourceStore>,
    feeds: Vec<Feed>,
    updates: BoxStream<'static, SessionUpdate>,
}

struct Feed {
    subscription: Subscription,
    snapshot_seen: bool,
}

impl ReleaseWatch {
    /// A watch on `cluster`. Without a store yet (not connected) it has no feeds and waits for
    /// the session's connection change.
    pub(super) fn open(source: Arc<dyn ReleaseSource>, cluster: &ClusterId) -> Self {
        let store = source.store(cluster);
        let selection = source.namespaces(cluster);
        let feeds = store
            .as_ref()
            .map(|store| Self::subscribe(store, &selection))
            .unwrap_or_default();
        Self {
            cluster: cluster.clone(),
            updates: source.session_updates(),
            source,
            selection,
            store,
            feeds,
        }
    }

    /// Moves the feeds to the cluster's current store when it is not the one they are on.
    fn follow_store(&mut self) {
        let current = self.source.store(&self.cluster);
        let same = match (&self.store, &current) {
            (Some(held), Some(now)) => held.is_same_store(now),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        // Drop the old feeds first so the forgotten store (and its connection) can go.
        self.feeds.clear();
        if let Some(store) = &current {
            self.feeds = Self::subscribe(store, &self.selection);
        }
        self.store = current;
    }

    /// Whether the watch currently has feeds on a store (tests read it).
    pub fn is_following_a_store(&self) -> bool {
        self.store.is_some()
    }

    fn subscribe(store: &ResourceStore, selection: &NamespaceSelection) -> Vec<Feed> {
        let selector = LabelSelector::parse("owner=helm").ok();
        kinds()
            .into_iter()
            .map(|gvk| {
                let scope = WatchScope::derive(selection, Scope::Namespaced);
                Feed {
                    subscription: store
                        .subscribe(StoreQuery::new(gvk, scope).with_selector(selector.clone())),
                    snapshot_seen: false,
                }
            })
            .collect()
    }

    /// Applies a session update; `true` when the list is stale because of it.
    fn follow(&mut self, update: SessionUpdate) -> bool {
        if update.cluster != self.cluster {
            return false;
        }
        match update.change {
            SessionChange::NamespaceChanged(selection) => {
                for feed in &mut self.feeds {
                    feed.subscription
                        .rescope(WatchScope::derive(&selection, Scope::Namespaced));
                }
                self.selection = selection;
                true
            }
            SessionChange::StateChanged { .. } | SessionChange::Closed => {
                self.follow_store();
                true
            }
            _ => false,
        }
    }

    /// The scope the feeds currently watch (tests read it).
    pub fn selection(&self) -> &NamespaceSelection {
        &self.selection
    }
}

impl Stream for ReleaseWatch {
    type Item = ();

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<()>> {
        let this = &mut *self;
        let mut changed = false;
        while let Poll::Ready(Some(update)) = this.updates.as_mut().poll_next(cx) {
            changed |= this.follow(update);
        }
        for feed in &mut this.feeds {
            // Everything pending is one coalesced delta per poll.
            while let Poll::Ready(Some(delta)) = Pin::new(&mut feed.subscription).poll_next(cx) {
                // A state-only item (loading, ready, a reconnecting feed) moves no record.
                if matches!(delta.rows, RowChange::Unchanged) {
                    continue;
                }
                if std::mem::replace(&mut feed.snapshot_seen, true) {
                    changed = true;
                }
            }
        }
        if changed {
            Poll::Ready(Some(()))
        } else {
            Poll::Pending
        }
    }
}
