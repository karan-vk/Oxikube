//! The status buckets of a count (E12-S07): they move in step with the deltas the feed applies,
//! one count per add, update and delete, and a bucket transition moves exactly one object.

use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::view::Bucket;
use oxikube_ports::Delta;
use oxikube_testkit::{daemonset, pod};

use super::*;
use crate::store::{CountState, CountTarget};

fn pod_target() -> CountTarget {
    CountTarget::core("", "pods").expect("pods are a core target")
}

fn phased(name: &str, phase: &str) -> Resource {
    let builder = pod().namespace("x").name(name);
    match phase {
        "Running" => builder.running(),
        "Succeeded" => builder.succeeded(),
        "Failed" => builder.failed(),
        _ => builder.pending(),
    }
    .build()
}

/// The pod buckets of `state`, as `(running, pending, failed, succeeded)`.
fn spread(state: &CountState) -> (usize, usize, usize, usize) {
    let count = state.count().expect("counted");
    let get = |b| count.buckets.get(b);
    (
        get(Bucket::Running),
        get(Bucket::Pending),
        get(Bucket::Failed),
        get(Bucket::Succeeded),
    )
}

#[test]
fn buckets_follow_adds_transitions_and_deletes() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![
            phased("a", "Running"),
            phased("b", "Pending"),
            phased("c", "Succeeded"),
        ])]),
        batch(vec![Delta::Applied(phased("b", "Running"))]),
        batch(vec![
            Delta::Applied(phased("d", "Failed")),
            Delta::Deleted(phased("a", "Running")),
        ]),
    ]));
    let lease = h
        .store
        .lease_counts(vec![pod_target()], &NamespaceSelection::All);
    h.settle();
    assert_eq!(spread(&lease.counts()[0]), (1, 1, 0, 1));
    h.advance(1);
    assert_eq!(
        spread(&lease.counts()[0]),
        (2, 0, 0, 1),
        "Pending to Running moves one count"
    );
    h.advance(1);
    assert_eq!(
        spread(&lease.counts()[0]),
        (1, 0, 1, 1),
        "one failed pod added, one running deleted"
    );
    let count = lease.counts()[0].count().expect("counted");
    let sum: usize = Bucket::ALL.iter().map(|b| count.buckets.get(*b)).sum();
    assert_eq!(sum, count.rated, "the buckets add up to the rated objects");
}

#[test]
fn a_namespace_part_of_a_wider_feed_counts_its_own_buckets() {
    let mut h = Harness::with_objects([phased("a", "Running"), phased("b", "Pending"), {
        let mut other = phased("c", "Failed");
        other.meta.namespace = Some("y".into());
        other
    }]);
    let _table = h.subscribe(all(pods()));
    let count = h
        .store
        .count(&pod_target(), &NamespaceSelection::single("x"))
        .count()
        .expect("counted");
    assert_eq!(count.buckets.get(Bucket::Running), 1);
    assert_eq!(count.buckets.get(Bucket::Pending), 1);
    assert_eq!(count.buckets.get(Bucket::Failed), 0, "the other namespace");
}

#[test]
fn daemonsets_are_ready_or_not_ready_by_their_ready_count() {
    let mut h = Harness::with_objects([
        daemonset()
            .namespace("x")
            .name("ok")
            .replicas(2)
            .ready(2)
            .build(),
        daemonset()
            .namespace("x")
            .name("short")
            .replicas(3)
            .ready(1)
            .build(),
    ]);
    let target = CountTarget::core("apps", "daemonsets").expect("a core target");
    let lease = h.store.lease_counts(vec![target], &NamespaceSelection::All);
    h.settle();
    let count = lease.counts()[0].count().expect("counted");
    assert_eq!(count.buckets.get(Bucket::Ready), 1);
    assert_eq!(count.buckets.get(Bucket::NotReady), 1);
}
