//! The budget hook: refusals, eviction of idle feeds, degrade, release.

use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;
use oxikube_testkit::ResourceCall;
use parking_lot::Mutex;

use super::*;
use crate::store::{
    Admission, FeedBudget, FeedKind, FeedPriority, FeedRequest, FeedScope, FeedState, MaxFeeds,
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

#[test]
fn a_refused_feed_is_admitted_when_another_view_subscribes_after_room_freed() {
    let budget = Arc::new(MaxFeeds::new(1));
    let mut h = Harness::with_options(StoreOptions {
        budget: budget.clone(),
        ..options_with_grace(0)
    });
    h.resources.insert(p("x", "a", "1"));
    let pod_sub = h.subscribe(all(pods()));
    let mut refused = h.subscribe(all(services()));
    assert!(matches!(
        refused.state(),
        FeedState::Failed {
            kind: ErrorKind::BudgetExceeded,
            ..
        }
    ));
    drop(pod_sub);
    h.settle();
    assert_eq!(budget.released_count(), 1, "the pod feed freed its slot");

    // A second view of the same kind while the refused one is still alive asks again.
    let second = h.subscribe(all(services()));
    assert_eq!(second.state(), FeedState::Ready);
    assert_eq!(refused.state(), FeedState::Ready, "the first view recovers");
    assert_eq!(h.resources.live_watches(), 1);
    let mut m = Mirror::default();
    m.drain(&mut refused);
    assert_eq!(m.last.as_ref().unwrap().state, FeedState::Ready);
}

#[test]
fn narrowing_at_the_budget_limit_swaps_the_feed_instead_of_refusing() {
    let budget = Arc::new(MaxFeeds::new(1));
    let mut h = Harness::with_options(options(budget.clone()));
    h.resources.insert(p("a", "1", "1"));
    h.resources.insert(p("b", "2", "1"));
    let mut sub = h.subscribe(all(pods()));

    sub.rescope(WatchScope::Namespaces(vec!["a".into()]));
    h.settle();
    assert_eq!(sub.state(), FeedState::Ready, "not refused by the budget");
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.names(), ["a/1"]);
    assert_eq!(h.resources.live_watches(), 1, "the cluster feed made room");
    let scopes: Vec<FeedScope> = h.store.feeds().into_iter().map(|f| f.key.scope).collect();
    assert_eq!(scopes, [FeedScope::Namespace("a".into())]);
    assert_eq!(budget.released_count(), 1);
}

#[test]
fn swapping_namespaces_at_the_budget_limit_keeps_every_part_admitted() {
    let budget = Arc::new(MaxFeeds::new(2));
    let mut h = Harness::with_options(options(budget));
    for (ns, name) in [("a", "1"), ("b", "2"), ("c", "3")] {
        h.resources.insert(p(ns, name, "1"));
    }
    let mut sub = h.subscribe(in_namespaces(pods(), &["a", "b"]));
    sub.rescope(WatchScope::Namespaces(vec!["b".into(), "c".into()]));
    h.settle();
    assert_eq!(sub.state(), FeedState::Ready);
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.names(), ["b/2", "c/3"]);
    assert_eq!(h.resources.live_watches(), 2);
}

/// A budget whose idle grace changes at run time (a settings hot reload, E04-F543).
struct Grace(Mutex<u64>);

impl FeedBudget for Grace {
    fn admit(&self, _: &FeedRequest, _: usize) -> Admission {
        Admission::Granted
    }

    fn released(&self, _: &FeedRequest) {}

    fn idle_grace(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(*self.0.lock()))
    }
}

#[test]
fn the_budgets_idle_grace_is_read_each_time_a_feed_goes_idle() {
    let budget = Arc::new(Grace(Mutex::new(5)));
    let mut h = Harness::with_options(options(budget.clone()));
    drop(h.subscribe(all(pods())));
    h.advance(4);
    assert_eq!(
        h.resources.live_watches(),
        1,
        "within the budget's 5 s, not the store's 30 s"
    );
    h.advance(2);
    assert_eq!(
        h.resources.live_watches(),
        0,
        "torn down after the budget's grace"
    );

    *budget.0.lock() = 60;
    drop(h.subscribe(all(pods())));
    h.advance(45);
    assert_eq!(
        h.resources.live_watches(),
        1,
        "the new grace applies to the next idle feed"
    );
    h.advance(20);
    assert_eq!(h.resources.live_watches(), 0);
}

/// Refuses everything and counts what the store gave up.
#[derive(Default)]
struct Refusing(Mutex<Vec<Gvk>>);

impl FeedBudget for Refusing {
    fn admit(&self, _: &FeedRequest, _: usize) -> Admission {
        Admission::Refused("full".into())
    }

    fn released(&self, _: &FeedRequest) {}

    fn refused(&self, request: &FeedRequest) {
        self.0.lock().push(request.key.gvk.clone());
    }
}

#[test]
fn the_budget_is_told_once_when_the_store_gives_a_feed_up() {
    let budget = Arc::new(Refusing::default());
    let mut h = Harness::with_options(options(budget.clone()));
    let sub = h.subscribe(all(pods()));
    assert!(matches!(
        sub.state(),
        FeedState::Failed {
            kind: ErrorKind::BudgetExceeded,
            ..
        }
    ));
    assert_eq!(*budget.0.lock(), [pods()]);
}
