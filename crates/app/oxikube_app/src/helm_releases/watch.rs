//! [`ReleaseWatch`]: one `()` whenever the cluster's release records or the view's scope change.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use futures::stream::BoxStream;
use oxikube_domain::ids::{ClusterId, Gvk, Scope};
use oxikube_domain::session::{NamespaceSelection, WatchScope};

use super::source::ReleaseSource;
use crate::session::{SessionChange, SessionUpdate};
use crate::store::{LabelSelector, ResourceStore, StoreQuery, Subscription};

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
/// list. The first message of each feed (its snapshot) is not a change and is swallowed. There
/// is no timer: with no change the stream stays pending.
///
/// Items are not rate limited; the view debounces them.
pub struct ReleaseWatch {
    cluster: ClusterId,
    selection: NamespaceSelection,
    feeds: Vec<Feed>,
    updates: BoxStream<'static, SessionUpdate>,
}

struct Feed {
    subscription: Subscription,
    snapshot_seen: bool,
}

impl ReleaseWatch {
    /// A watch on `cluster`, `None` while the cluster has no store (not connected).
    pub(super) fn open(source: &dyn ReleaseSource, cluster: &ClusterId) -> Option<Self> {
        let store = source.store(cluster)?;
        let selection = source.namespaces(cluster);
        Some(Self {
            cluster: cluster.clone(),
            feeds: Self::subscribe(&store, &selection),
            selection,
            updates: source.session_updates(),
        })
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
            SessionChange::StateChanged { .. } | SessionChange::Closed => true,
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
            while let Poll::Ready(Some(_)) = Pin::new(&mut feed.subscription).poll_next(cx) {
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
