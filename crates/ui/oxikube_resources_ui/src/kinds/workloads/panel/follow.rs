//! Following the owned objects: the ReplicaSet and Pod subscriptions, their polling, and the rows
//! recomputed when a delta lands.

use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use futures::Stream as _;
use gpui::{AsyncApp, Context, Task, WeakEntity};
use oxikube_app::store::{LabelSelector, ResourceStore, StoreDelta, StoreFilter, StoreQuery};
use oxikube_domain::view::WorkloadKind;
use oxikube_runtime::notify_coalesced;

use super::{Followed, OwnedFeed, Which, WorkloadPanel, owned};
use crate::detail::DetailView;

impl WorkloadPanel {
    /// Starts (or restarts) the owned feeds for the current object and store.
    pub(super) fn follow(
        &mut self,
        store: Option<ResourceStore>,
        detail: &gpui::Entity<DetailView>,
        cx: &mut Context<Self>,
    ) {
        let (Some(model), Some(store)) = (&self.model, store) else {
            return;
        };
        let now = Followed {
            store: store.clone(),
            uid: model.uid.clone(),
            selector: model.selector.clone(),
        };
        let same = self.followed.as_ref().is_some_and(|old| {
            old.store.is_same(&now.store) && old.uid == now.uid && old.selector == now.selector
        });
        if same || now.uid.is_none() {
            return;
        }
        let (target, kind) = {
            let view = detail.read(cx);
            (view.target().clone(), model.kind)
        };
        let namespace = target.namespace().map(str::to_owned);
        let label_filter = LabelSelector::parse(&now.selector)
            .ok()
            .filter(|s| !s.is_empty())
            .map(StoreFilter::labels)
            .unwrap_or_default();
        let subscribe = |gvk: oxikube_domain::ids::Gvk, cx: &Context<Self>| {
            let scope = detail
                .read(cx)
                .scope_for(&store, &gvk, namespace.as_deref());
            store.subscribe(StoreQuery::new(gvk, scope).with_filter(label_filter.clone()))
        };
        self.replicasets = OwnedFeed::default();
        self.pods = OwnedFeed::default();
        if kind == WorkloadKind::Deployment {
            self.replicasets.subscription = Some(subscribe(owned::replicaset_gvk(), cx));
            self.replicasets.task = Some(Self::poll(Which::ReplicaSets, cx));
        }
        self.pods.subscription = Some(subscribe(owned::pod_gvk(), cx));
        self.pods.task = Some(Self::poll(Which::Pods, cx));
        self.followed = Some(now);
        self.recompute(cx);
    }

    /// The task that polls one owned subscription until the panel goes.
    fn poll(which: Which, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            futures::future::poll_fn(|task_cx| {
                match this.update(cx, |panel, cx| panel.drain(which, task_cx, cx)) {
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
        which: Which,
        task_cx: &mut std::task::Context<'_>,
        cx: &mut Context<Self>,
    ) -> bool {
        let feed = match which {
            Which::ReplicaSets => &mut self.replicasets,
            Which::Pods => &mut self.pods,
        };
        let Some(subscription) = feed.subscription.as_mut() else {
            return false;
        };
        let mut deltas: Vec<StoreDelta> = Vec::new();
        let live = loop {
            match Pin::new(&mut *subscription).poll_next(task_cx) {
                Poll::Ready(Some(delta)) => deltas.push(delta),
                Poll::Ready(None) => break false,
                Poll::Pending => break true,
            }
        };
        if !deltas.is_empty() {
            for delta in deltas {
                delta.apply_to(&mut feed.items);
            }
            self.recompute(cx);
            notify_coalesced(cx);
        }
        live
    }

    /// Rebuilds the two sections' rows from the feeds' objects.
    fn recompute(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (cluster, columns, uid) = {
            let view = detail.read(cx);
            (
                view.target().cluster.clone(),
                view.deps().columns.clone(),
                self.model.as_ref().and_then(|m| m.uid.clone()),
            )
        };
        let Some(uid) = uid else { return };
        let mut owners: HashSet<Arc<str>> = HashSet::from([uid]);
        let kind = self.model.as_ref().map(|m| m.kind);
        if kind == Some(WorkloadKind::Deployment) {
            let sets = owned::owned_by(&owners, &self.replicasets.items);
            self.replicaset_rows = owned::replicaset_rows(&cluster, &sets);
            owners = owned::uids(&sets);
        }
        let pods = owned::owned_by(&owners, &self.pods.items);
        self.pod_rows = owned::pod_rows(&cluster, &columns, &pods, jiff::Timestamp::now());
    }
}
