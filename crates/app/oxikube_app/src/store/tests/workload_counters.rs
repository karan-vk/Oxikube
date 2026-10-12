//! [`WorkloadCounters`] over a fake store (E12-S07): it opens the feeds of its kinds, reports a
//! change only when a number moved, releases the feeds when told to or when dropped, and follows
//! a namespace selection and a reconnect.

use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::view::Bucket;
use oxikube_ports::Delta;

use super::*;
use crate::store::{CountState, CountTarget, WorkloadCounters};

fn targets() -> Vec<CountTarget> {
    vec![
        CountTarget::core("", "pods").expect("pods"),
        CountTarget::core("apps", "deployments").expect("deployments"),
    ]
}

fn pod_in(ns: &str, name: &str, ready: bool) -> Resource {
    let builder = pod().namespace(ns).name(name);
    if ready {
        builder.running().build()
    } else {
        builder.pending().build()
    }
}

fn running_pods(counters: &WorkloadCounters) -> usize {
    counters.states()[0]
        .count()
        .map_or(0, |c| c.buckets.get(Bucket::Running))
}

#[test]
fn nothing_is_watched_until_it_follows_a_store() {
    let counters = WorkloadCounters::new(targets());
    assert_eq!(
        counters.states(),
        [CountState::NotWatched, CountState::NotWatched]
    );
    assert!(!counters.is_open());
}

#[test]
fn it_opens_the_feeds_and_reports_a_change_only_when_a_number_moved() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![
            pod_in("x", "a", true),
            pod_in("x", "b", false),
        ])]),
        batch(vec![Delta::Applied(pod_in("x", "b", true))]),
    ]));
    let mut counters = WorkloadCounters::new(targets());
    let selection = NamespaceSelection::All;
    counters.follow(Some((&h.store, &selection)));
    assert!(counters.is_open());
    assert_eq!(h.resources.live_watches(), 0, "feeds start on the spawner");
    h.settle();
    assert!(counters.refresh(), "the first list moved the numbers");
    assert_eq!(running_pods(&counters), 1);
    assert!(!counters.refresh(), "nothing moved: no redraw");
    h.advance(1);
    assert!(counters.refresh());
    assert_eq!(
        running_pods(&counters),
        2,
        "Pending to Running moved one pod"
    );
    assert_eq!(
        h.resources.live_watches(),
        2,
        "pods and deployments, one feed each"
    );
}

#[test]
fn releasing_stops_the_feeds_after_the_grace_period() {
    let mut h = Harness::with_options(options_with_grace(10));
    let mut counters = WorkloadCounters::new(targets());
    counters.follow(Some((&h.store, &NamespaceSelection::All)));
    h.settle();
    assert_eq!(h.resources.live_watches(), 2);
    counters.release();
    assert!(!counters.is_open());
    assert_eq!(
        counters.states(),
        [CountState::NotWatched, CountState::NotWatched]
    );
    h.advance(11);
    assert_eq!(
        h.resources.live_watches(),
        0,
        "the overview closed: no feed runs"
    );
}

#[test]
fn dropping_the_counters_releases_the_feeds_too() {
    let mut h = Harness::with_options(options_with_grace(0));
    let mut counters = WorkloadCounters::new(targets());
    counters.follow(Some((&h.store, &NamespaceSelection::All)));
    h.settle();
    assert_eq!(h.resources.live_watches(), 2);
    drop(counters);
    h.settle();
    assert_eq!(h.resources.live_watches(), 0);
}

#[test]
fn following_nothing_goes_back_to_not_watched() {
    let mut h = Harness::with_options(options_with_grace(0));
    h.resources.insert(pod_in("x", "a", true));
    let mut counters = WorkloadCounters::new(targets());
    counters.follow(Some((&h.store, &NamespaceSelection::All)));
    h.settle();
    counters.refresh();
    assert_eq!(running_pods(&counters), 1);
    assert!(counters.follow(None), "disconnected: the numbers go");
    assert_eq!(
        counters.states(),
        [CountState::NotWatched, CountState::NotWatched]
    );
    assert!(!counters.follow(None), "and stay gone without a change");
    h.settle();
    assert_eq!(h.resources.live_watches(), 0);
}

#[test]
fn it_follows_the_namespace_selection_on_the_same_store() {
    let mut h = Harness::with_options(options_with_grace(0));
    for object in [
        pod_in("a", "p1", true),
        pod_in("a", "p2", false),
        pod_in("b", "p3", true),
    ] {
        h.resources.insert(object);
    }
    let mut counters = WorkloadCounters::new(targets());
    counters.follow(Some((&h.store, &NamespaceSelection::single("a"))));
    h.settle();
    counters.refresh();
    assert_eq!(running_pods(&counters), 1);
    counters.follow(Some((
        &h.store,
        &NamespaceSelection::from_names(["a", "b"]),
    )));
    h.settle();
    counters.refresh();
    assert_eq!(running_pods(&counters), 2, "b joined");
}

#[test]
fn a_different_kind_list_starts_over() {
    let mut counters = WorkloadCounters::new(targets());
    assert!(
        !counters.set_targets(targets()),
        "the same list changes nothing"
    );
    let one = vec![CountTarget::core("batch", "jobs").expect("jobs")];
    assert!(counters.set_targets(one));
    assert_eq!(counters.states().len(), 1);
}
