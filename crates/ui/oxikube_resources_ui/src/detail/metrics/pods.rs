//! A workload's pods, followed to know which names to query.
//!
//! The Metrics tab of a Deployment, StatefulSet, DaemonSet or ReplicaSet draws one line per pod
//! it selects. The pods come from the namespace's Pod feed filtered by the workload's label
//! selector (the feed other views of those pods share), started when the tab is first shown and
//! dropped with it. Deltas are applied off the render path; the names are read when a fetch
//! starts.

use std::pin::Pin;
use std::task::Poll;

use futures::Stream as _;
use gpui::{AsyncApp, Context, Task, WeakEntity};
use oxikube_app::store::{FeedState, LabelSelector, StoreDelta, StoreFilter, StoreQuery};
use oxikube_domain::ids::Gvk;
use oxikube_domain::view::WorkloadDetail;

use crate::detail::view::DetailView;

impl DetailView {
    /// Starts following the pods the workload selects, once per store and selector. Does nothing
    /// until the object is known (its arrival calls [`DetailView::metrics_object_changed`]).
    pub(super) fn follow_metric_pods(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.connected_store() else {
            return;
        };
        let Some(selector) = self
            .resource()
            .and_then(|res| WorkloadDetail::from_resource(res).ok())
            .map(|detail| detail.selector.display())
        else {
            return;
        };
        let same = self.metrics.pods.selector.as_deref() == Some(selector.as_str())
            && self
                .metrics
                .pods
                .store
                .as_ref()
                .is_some_and(|old| old.is_same(&store))
            && self.metrics.pods.subscription.is_some();
        if same {
            return;
        }
        // A workload that selects nothing in particular selects no pods of its own.
        let filter = match LabelSelector::parse(&selector) {
            Ok(parsed) if !parsed.is_empty() => StoreFilter::labels(parsed),
            _ => {
                self.metrics.pods = Default::default();
                self.metrics.pods.selector = Some(selector);
                self.metrics.pods.ready = true;
                return;
            }
        };
        let pods = Gvk::new("", "v1", "Pod");
        let scope = self.scope_for(&store, &pods, self.target.namespace.as_deref());
        self.metrics.pods = Default::default();
        self.metrics.pods.subscription =
            Some(store.subscribe(StoreQuery::new(pods, scope).with_filter(filter)));
        self.metrics.pods.store = Some(store);
        self.metrics.pods.selector = Some(selector);
        self.metrics.pods.task = Some(Self::poll_metric_pods(cx));
    }

    /// The names of the pods followed, for the queries. Pods being deleted are left out.
    pub(in crate::detail) fn metric_pod_names(&self) -> Vec<String> {
        self.metrics
            .pods
            .items
            .iter()
            .filter(|object| !object.meta().is_terminating())
            .map(|object| object.name().to_owned())
            .collect()
    }

    fn poll_metric_pods(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            futures::future::poll_fn(|task_cx| {
                match this.update(cx, |view, cx| view.drain_metric_pods(task_cx, cx)) {
                    Ok(true) => Poll::Pending,
                    _ => Poll::Ready(()),
                }
            })
            .await;
        })
    }

    /// Applies every delta the pods subscription has ready; whether to keep polling.
    fn drain_metric_pods(
        &mut self,
        task_cx: &mut std::task::Context<'_>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(subscription) = self.metrics.pods.subscription.as_mut() else {
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
        if deltas.is_empty() {
            return live;
        }
        for delta in deltas {
            delta.apply_to(&mut self.metrics.pods.items);
            if matches!(delta.state, FeedState::Ready) {
                self.metrics.pods.ready = true;
            }
        }
        // The first answer, or the first pod of a workload that had none, starts the fetching;
        // a running loop reads the names itself at its next tick.
        self.metrics_object_changed(cx);
        live
    }
}
