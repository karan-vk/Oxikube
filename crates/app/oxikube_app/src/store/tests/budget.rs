//! The budget hook: refusals, eviction of idle feeds, degrade, release.

use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::ResourceCall;
use parking_lot::Mutex;

use super::*;
use crate::store::{
    Admission, FeedBudget, FeedKind, FeedPriority, FeedRequest, FeedState, MaxFeeds,
};

fn options(budget: Arc<dyn FeedBudget>) -> StoreOptions {
    StoreOptions {
        budget,
        ..options_with_grace(30)
    }
}

fn services() -> Gvk {
    Gvk::new("", "v1", "Service")
}

#[test]
fn a_refused_feed_fails_with_budget_exceeded_and_never_opens() {
    let budget = Arc::new(MaxFeeds::new(1));
    let mut h = Harness::with_options(options(budget.clone()));
    let _pods = h.subscribe(all(pods()));
    let refused = h.subscribe(all(services()));
    assert!(matches!(
        refused.state(),
        FeedState::Failed { kind: ErrorKind::BudgetExceeded, message } if message.contains("limit 1")
    ));
    assert_eq!(h.resources.live_watches(), 1);
    drop(refused);
    h.settle();
    assert_eq!(h.store.feeds().len(), 1, "a refused entry leaves at once");
    assert_eq!(budget.released_count(), 0, "never admitted, never released");
}

#[test]
fn an_idle_feed_is_evicted_to_make_room() {
    let budget = Arc::new(MaxFeeds::new(1));
    let mut h = Harness::with_options(options(budget.clone()));
    drop(h.subscribe(all(pods())));
    h.settle();
    assert_eq!(h.resources.live_watches(), 1, "in its grace period");

    let svc = h.subscribe(all(services()));
    assert_eq!(svc.state(), FeedState::Ready);
    assert_eq!(h.resources.live_watches(), 1, "the idle pod feed made room");
    assert_eq!(budget.released_count(), 1);
    let kinds: Vec<Gvk> = h.store.feeds().into_iter().map(|f| f.key.gvk).collect();
    assert_eq!(kinds, [services()]);
}

#[test]
fn high_priority_kinds_use_the_headroom() {
    let budget = Arc::new(MaxFeeds::new(1).with_headroom(1));
    let mut h = Harness::with_options(options(budget));
    let _services = h.subscribe(all(services()));
    let pod_sub = h.subscribe(all(pods()));
    assert_eq!(h.store.plan(&pods()).priority, FeedPriority::High);
    assert_eq!(pod_sub.state(), FeedState::Ready);
}

/// Degrades every full feed to metadata-only and records what it saw.
#[derive(Default)]
struct Degrading {
    seen: Mutex<Vec<(FeedRequest, usize)>>,
    released: Mutex<Vec<FeedRequest>>,
}

impl FeedBudget for Degrading {
    fn admit(&self, request: &FeedRequest, running: usize) -> Admission {
        self.seen.lock().push((request.clone(), running));
        if request.kind == FeedKind::Full {
            Admission::Degraded(FeedKind::Metadata)
        } else {
            Admission::Granted
        }
    }

    fn released(&self, request: &FeedRequest) {
        self.released.lock().push(request.clone());
    }
}

#[test]
fn a_degraded_feed_opens_metadata_only_and_is_released_once() {
    let budget = Arc::new(Degrading::default());
    let mut h = Harness::with_options(StoreOptions {
        budget: budget.clone(),
        ..options_with_grace(0)
    });
    h.resources.insert(p("x", "a", "1"));
    let mut sub = h.subscribe(all(pods()));
    assert_eq!(sub.feed_kind(), FeedKind::Metadata);
    assert!(
        h.resources
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, ResourceCall::Watch { options, .. } if options.metadata_only))
    );
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert!(m.rows[0].resource().unwrap().is_partial());
    assert_eq!(budget.seen.lock()[0].1, 0);
    drop(sub);
    h.settle();
    let released = budget.released.lock();
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].kind, FeedKind::Metadata);
}
