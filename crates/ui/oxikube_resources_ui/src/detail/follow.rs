//! Following the object: the store subscription, the full read for kinds whose feed carries
//! less than the whole object, and the owners' scopes.
//!
//! The object is a one-row subscription (exact name, its namespace) on the same feed the table
//! uses, so opening a drawer starts no extra watch. A feed that delivers metadata only (Secrets,
//! ConfigMaps) or Table rows (custom resources) leaves `spec` and `status` unknown; the view
//! reads the object once (`ResourceReader::get`, on the Tokio bridge, secret values removed on
//! the way) and again whenever the store reports a new version. Nothing here blocks: the
//! subscription is polled by one foreground task the view owns, each wake applies everything
//! pending and redraws through `notify_coalesced`.

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use futures::{Stream as _, StreamExt as _};
use gpui::{AsyncApp, Context, Task, WeakEntity};
use oxikube_app::columns::Tone;
use oxikube_app::store::{
    FeedScope, FeedState, ResourceStore, StoreDelta, StoreFilter, StoreObject, StoreQuery,
};
use oxikube_app::{ColumnId, TableColumns};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::session::WatchScope;
use oxikube_runtime::notify_coalesced;
use oxikube_workspace::ItemEvent;

use super::model::{DetailModel, Row, StatusChip};
use super::state::{DetailDeps, DetailState, FullState};
use super::view::DetailView;

/// Status cells tried for the chip, in order: the kind's own status, then what CRDs call it.
const STATUS_COLUMNS: [&str; 4] = ["status", "phase", "state", "ready"];

/// Which of the view's subscriptions a poll task serves.
#[derive(Clone, Copy)]
pub(super) enum Feed {
    Object,
    Events,
}

impl DetailView {
    /// The task that follows the cluster's session updates (a reconnect hands out a new store).
    pub(super) fn follow_session(
        target: &ResourceRef,
        deps: &DetailDeps,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let mut updates = deps.sessions.subscribe();
        let cluster = target.cluster.clone();
        cx.spawn(async move |this, cx| {
            while let Some(update) = updates.next().await {
                if update.as_ref().is_ok_and(|u| u.cluster != cluster) {
                    continue;
                }
                let followed = this.update(cx, |view, cx| {
                    view.resubscribe(cx);
                    view.restart_events(cx);
                });
                if followed.is_err() {
                    break;
                }
            }
        })
    }

    /// The scope that reaches `gvk` objects in `namespace` through feeds that are already
    /// running where possible: a cluster-wide feed of the kind when the store holds one (a table
    /// with every namespace selected), else just the namespace's, so opening one object never
    /// starts a cluster-wide watch of its kind. Cluster-scoped kinds have only the one feed.
    pub(super) fn scope_for(
        &self,
        store: &ResourceStore,
        gvk: &Gvk,
        namespace: Option<&str>,
    ) -> WatchScope {
        let Some(namespace) = namespace else {
            return WatchScope::Cluster;
        };
        let cluster_wide = store.feeds().iter().any(|feed| {
            feed.key.gvk == *gvk
                && feed.key.scope == FeedScope::Cluster
                && !feed.state.is_terminal()
        });
        if cluster_wide {
            WatchScope::Cluster
        } else {
            WatchScope::Namespaces(vec![namespace.to_owned()])
        }
    }

    /// The store of the cluster's session, while it is connected.
    pub(super) fn connected_store(&self) -> Option<ResourceStore> {
        let session = self.deps.sessions.get(&self.target.cluster)?;
        if !session.is_connected() {
            return None;
        }
        self.deps.stores.for_session(&session)
    }

    /// Subscribes to the object in the session's store. Does nothing while the session is not
    /// connected, or while the subscription is on the current store.
    pub(super) fn resubscribe(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.connected_store() else {
            return;
        };
        if self.store.as_ref().is_some_and(|s| s.is_same(&store)) && self.subscription.is_some() {
            return;
        }
        let namespace = self.target.namespace.clone();
        let filter = StoreFilter {
            name: Some(self.target.name.to_string()),
            namespaces: namespace
                .as_deref()
                .map(|ns| BTreeSet::from([ns.to_owned()])),
            ..StoreFilter::default()
        };
        let scope = self.scope_for(&store, &self.target.gvk, namespace.as_deref());
        let query = StoreQuery::new(self.target.gvk.clone(), scope).with_filter(filter);
        self.subscription = Some(store.subscribe(query));
        self.store = Some(store);
        self.feed.clear();
        self.owner_scopes.clear();
        // The new store re-delivers the object at the version we already have: a full read that
        // failed (or never started) is asked for again then.
        if matches!(self.full, FullState::Failed(_) | FullState::Idle) {
            self.refetch_full = true;
        }
        self.feed_task = Some(Self::poll(Feed::Object, cx));
    }

    /// The task that polls a subscription until the view goes.
    pub(super) fn poll(feed: Feed, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            futures::future::poll_fn(|task_cx| {
                match this.update(cx, |view, cx| view.drain(feed, task_cx, cx)) {
                    Ok(true) => Poll::Pending,
                    _ => Poll::Ready(()),
                }
            })
            .await;
        })
    }

    /// Applies every delta the subscription has ready; whether to keep polling.
    fn drain(
        &mut self,
        feed: Feed,
        task_cx: &mut std::task::Context<'_>,
        cx: &mut Context<Self>,
    ) -> bool {
        let subscription = match feed {
            Feed::Object => self.subscription.as_mut(),
            Feed::Events => self.events.subscription.as_mut(),
        };
        let Some(subscription) = subscription else {
            return false;
        };
        let mut deltas = Vec::new();
        let live = loop {
            match Pin::new(&mut *subscription).poll_next(task_cx) {
                Poll::Ready(Some(delta)) => deltas.push(delta),
                Poll::Ready(None) => break false,
                Poll::Pending => break true,
            }
        };
        if !deltas.is_empty() {
            for delta in deltas {
                match feed {
                    Feed::Object => self.apply(delta, cx),
                    Feed::Events => self.apply_events(delta, cx),
                }
            }
            notify_coalesced(cx);
        }
        live
    }

    /// Applies one delta of the object's subscription.
    pub(super) fn apply(&mut self, delta: StoreDelta, cx: &mut Context<Self>) {
        if let Some(columns) = &delta.columns {
            self.provider = Arc::new(TableColumns::new(
                &columns.columns,
                columns.source,
                self.target.scope(),
            ));
        }
        delta.apply_to(&mut self.feed);
        let found = self.feed.first().cloned();
        let state = match (&found, &delta.state) {
            (Some(_), _) => DetailState::Live,
            (None, FeedState::Ready) if self.object.is_some() => DetailState::Deleted,
            (None, FeedState::Ready) => DetailState::NotFound,
            (None, FeedState::Forbidden { message } | FeedState::Failed { message, .. }) => {
                DetailState::Unavailable(message.clone())
            }
            (None, _) => self.state.clone(),
        };
        let changed_state = state != self.state;
        self.state = state;
        match found {
            Some(object) => self.set_object(object, cx),
            None if changed_state => cx.notify(),
            None => {}
        }
    }

    /// Takes a new version of the object: rebuilds the model, and reads the full object when
    /// the feed's is partial.
    fn set_object(&mut self, object: Arc<StoreObject>, cx: &mut Context<Self>) {
        let version_changed = self
            .object
            .as_ref()
            .is_none_or(|old| old.meta().resource_version != object.meta().resource_version);
        let first = self.object.is_none();
        let refetch = std::mem::take(&mut self.refetch_full);
        self.object = Some(object);
        if first || version_changed || refetch {
            if self.needs_full() {
                self.fetch_full(cx);
            }
            self.rebuild(cx);
            if first {
                cx.emit(ItemEvent::UpdateTab);
            }
        } else {
            // The version is the same but the feed may have recomputed cells (a Table row's
            // age): the chip follows them.
            self.rebuild(cx);
        }
    }

    /// Whether the store's object lacks `spec` and `status`.
    pub(super) fn needs_full(&self) -> bool {
        match self.object.as_deref() {
            Some(StoreObject::Resource(resource)) => resource.is_partial(),
            Some(StoreObject::Row(_)) => true,
            None => false,
        }
    }

    /// The status chip: the kind's status cell through the same provider the table uses.
    fn chip(&self, object: &StoreObject) -> Option<StatusChip> {
        let now = self.now();
        STATUS_COLUMNS.iter().find_map(|id| {
            let cell = self.provider.cell(object, &ColumnId::new(*id), now);
            let text = cell.display();
            (!text.is_empty()).then(|| StatusChip {
                text: text.to_owned(),
                tone: cell.tone(),
            })
        })
    }

    /// Rebuilds the model and the Overview's rows from the current object, keeping the scroll.
    pub(super) fn rebuild(&mut self, cx: &mut Context<Self>) {
        let Some(object) = self.object.clone() else {
            return;
        };
        let chip = self.chip(&object).or_else(|| {
            object.meta().is_terminating().then(|| StatusChip {
                text: "Terminating".to_owned(),
                tone: Tone::Warn,
            })
        });
        let model = DetailModel::build(&object, &self.target.gvk, self.full.resource(), chip);
        let rows = model.rows();
        self.sync_list(&rows);
        self.body = rows;
        self.model = Some(model);
        self.resolve_owners(cx);
        cx.notify();
    }

    /// Tells the list state which rows changed: the middle that differs is spliced, the rest is
    /// remeasured in place, so the scroll position stays where the user left it.
    fn sync_list(&self, new: &[Row]) {
        let old = &self.body;
        let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        if old.len() != new.len() || prefix + suffix < old.len() {
            self.overview
                .splice(prefix..old.len() - suffix, new.len() - prefix - suffix);
        }
        if !new.is_empty() {
            self.overview.remeasure_items(0..new.len());
        }
    }
}
