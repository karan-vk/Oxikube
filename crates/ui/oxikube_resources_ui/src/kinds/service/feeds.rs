//! The two feeds of a Service's panel: its EndpointSlices and its pods, both server-filtered by
//! label ([`ServiceBackends`]). They start when the panel first sees the Service and stop when the
//! panel is dropped (the detail closed): a table of Services starts none.

use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use futures::Stream as _;
use oxikube_app::store::{FeedState, ResourceStore, StoreDelta, StoreObject, Subscription};
use oxikube_app::{PodsPlan, ServiceBackends};

use super::model::FeedStatus;

/// What the feeds hold. Cheap to read; all the work happens in [`Feeds::drain`].
#[derive(Default)]
pub(super) struct Feeds {
    store: Option<ResourceStore>,
    /// The Service the subscriptions are for: namespace, name and selector text.
    key: Option<(Option<Arc<str>>, Arc<str>, String)>,
    slices_sub: Option<Subscription>,
    pods_sub: Option<Subscription>,
    pub(super) slices: Vec<Arc<StoreObject>>,
    pub(super) pods: Vec<Arc<StoreObject>>,
    pub(super) slices_status: FeedStatus,
    pub(super) pods_status: FeedStatus,
    /// Whether the Service selects pods (a pod feed exists, or is being waited for).
    pub(super) selects_pods: bool,
}

impl Feeds {
    /// Subscribes to `backends` of the Service `key` on `store`, unless that is what the feeds
    /// follow already. Returns whether anything changed (the caller starts polling).
    pub(super) fn follow(
        &mut self,
        store: ResourceStore,
        key: (Option<Arc<str>>, Arc<str>, String),
        backends: &ServiceBackends,
    ) -> bool {
        let same_store = self.store.as_ref().is_some_and(|s| s.is_same(&store));
        if same_store && self.key.as_ref() == Some(&key) && self.slices_sub.is_some() {
            return false;
        }
        self.slices_sub = Some(store.subscribe(backends.slices.clone()));
        self.slices.clear();
        self.slices_status = FeedStatus::Loading;
        match &backends.pods {
            PodsPlan::Query(query) => {
                self.pods_sub = Some(store.subscribe(query.clone()));
                self.selects_pods = true;
            }
            PodsPlan::None(_) => {
                self.pods_sub = None;
                self.selects_pods = false;
            }
        }
        self.pods.clear();
        self.pods_status = FeedStatus::Loading;
        self.store = Some(store);
        self.key = Some(key);
        true
    }

    /// Applies every delta that is ready. Returns `(changed, live)`: whether a list or a status
    /// changed, and whether to keep polling.
    pub(super) fn drain(&mut self, task_cx: &mut std::task::Context<'_>) -> (bool, bool) {
        let mut changed = false;
        let mut live = false;
        if let Some(sub) = self.slices_sub.as_mut() {
            let (c, l) = drain_one(sub, &mut self.slices, &mut self.slices_status, task_cx);
            (changed, live) = (changed | c, live | l);
        }
        if let Some(sub) = self.pods_sub.as_mut() {
            let (c, l) = drain_one(sub, &mut self.pods, &mut self.pods_status, task_cx);
            (changed, live) = (changed | c, live | l);
        }
        (changed, live)
    }

    /// Drops the subscriptions (the feeds stop after the store's grace period).
    pub(super) fn release(&mut self) {
        self.slices_sub = None;
        self.pods_sub = None;
        self.store = None;
        self.key = None;
    }
}

fn drain_one(
    sub: &mut Subscription,
    rows: &mut Vec<Arc<StoreObject>>,
    status: &mut FeedStatus,
    task_cx: &mut std::task::Context<'_>,
) -> (bool, bool) {
    let mut changed = false;
    let live = loop {
        match Pin::new(&mut *sub).poll_next(task_cx) {
            Poll::Ready(Some(delta)) => {
                changed |= apply(&delta, rows, status);
            }
            Poll::Ready(None) => break false,
            Poll::Pending => break true,
        }
    };
    (changed, live)
}

fn apply(delta: &StoreDelta, rows: &mut Vec<Arc<StoreObject>>, status: &mut FeedStatus) -> bool {
    delta.apply_to(rows);
    let next = match &delta.state {
        FeedState::Ready => FeedStatus::Ready,
        FeedState::Forbidden { message }
        | FeedState::Unauthorized { message }
        | FeedState::Failed { message, .. } => FeedStatus::Failed(message.clone()),
        FeedState::Warming | FeedState::Retrying { .. } => status.clone(),
    };
    let moved = next != *status;
    *status = next;
    // Rows moved, or the state did: a delta is only sent when something did.
    moved || !matches!(delta.rows, oxikube_app::store::RowChange::Unchanged)
}
