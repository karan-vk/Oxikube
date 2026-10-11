//! Pods by node (E12-S09): the index follows add, move and delete deltas, answers only once the
//! cluster-wide Pod feed is ready, costs nothing while nobody leases it, and a delta costs O(1).

use std::time::Instant;

use oxikube_domain::Demand;
use oxikube_ports::Delta;
use oxikube_testkit::pod;

use super::*;
use crate::store::nodes::PodsByNode;

/// A running pod `ns/name` on `node` requesting 100m / 128Mi (the builder's default).
fn on(node: &str, ns: &str, name: &str, rv: &str) -> Resource {
    let mut r = pod().namespace(ns).name(name).node(node).running().build();
    r.meta.resource_version = Some(rv.into());
    r
}

fn demand(pods: u32) -> Demand {
    Demand {
        pods,
        cpu_requests: 100 * i64::from(pods),
        cpu_limits: 0,
        memory_requests: 128 * 1024 * 1024 * i64::from(pods),
        memory_limits: 0,
    }
}

#[test]
fn the_index_follows_add_move_and_delete_deltas() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![
            on("n1", "x", "a", "1"),
            on("n1", "x", "b", "1"),
            on("n2", "y", "c", "1"),
        ])]),
        // A pod is added, one moves from n1 to n2 (a recreated pod keeps its name), one is gone.
        batch(vec![
            Delta::Applied(on("n2", "y", "d", "1")),
            Delta::Applied(on("n2", "x", "a", "2")),
            Delta::Deleted(on("n1", "x", "b", "1")),
        ]),
        // A pod finishes: it stops counting.
        batch(vec![Delta::Applied({
            let mut done = pod()
                .namespace("y")
                .name("c")
                .node("n2")
                .succeeded()
                .build();
            done.meta.resource_version = Some("2".into());
            done
        })]),
    ]));
    let lease = h.store.lease_node_loads();
    h.settle();
    assert!(lease.is_ready());
    assert_eq!(lease.demand("n1"), Some(demand(2)));
    assert_eq!(lease.demand("n2"), Some(demand(1)));
    assert_eq!(
        lease.demand("n3"),
        Some(Demand::ZERO),
        "a node with no pods"
    );

    h.advance(1);
    assert_eq!(lease.demand("n1"), Some(demand(0)));
    assert_eq!(lease.demand("n2"), Some(demand(3)), "c, d and the moved a");
    let names = names(&lease.pods("n2").unwrap());
    assert_eq!(names, ["x/a", "y/c", "y/d"]);

    h.advance(1);
    assert_eq!(lease.demand("n2"), Some(demand(2)), "the finished pod left");
    assert_eq!(names_of(&lease, "n2"), ["x/a", "y/d"]);
}

fn names_of(lease: &crate::store::NodeLoadsLease, node: &str) -> Vec<String> {
    names(&lease.pods(node).unwrap())
}

#[test]
fn nothing_answers_until_the_feed_is_ready_or_without_a_lease() {
    let mut h = Harness::with_objects([on("n1", "x", "a", "1")]);
    let lease = h.store.lease_node_loads();
    assert!(!lease.is_ready(), "the feed has not listed yet");
    assert_eq!(lease.demand("n1"), None);
    assert_eq!(lease.pods("n1"), None);
    h.settle();
    assert_eq!(lease.demand("n1"), Some(demand(1)));
}

#[test]
fn the_index_is_off_without_a_lease_and_built_from_the_cache_when_one_arrives() {
    let mut h = Harness::with_objects([on("n1", "x", "a", "1"), on("n1", "x", "b", "1")]);
    // A Pods table holds the feed; nobody asked for the index, so nothing is indexed.
    let _table = h.subscribe(all(pods()));
    let key = crate::store::FeedKey::new(pods(), crate::store::FeedScope::Cluster);
    let indexed = |h: &Harness| {
        h.store
            .inner()
            .with_entry(&key, |st| st.cache.pods_by_node().map(PodsByNode::len))
            .flatten()
    };
    assert_eq!(indexed(&h), None);

    let lease = h.store.lease_node_loads();
    assert_eq!(
        indexed(&h),
        None,
        "the caller does not wait for the build: it runs on the spawner"
    );
    assert_eq!(lease.demand("n1"), None, "and reads wait for it");
    h.settle();
    assert_eq!(
        indexed(&h),
        Some(2),
        "built from the cached pods in one pass"
    );
    assert_eq!(lease.demand("n1"), Some(demand(2)));
    let second = h.store.lease_node_loads();
    drop(lease);
    assert_eq!(indexed(&h), Some(2), "another lease keeps it");
    drop(second);
    assert_eq!(indexed(&h), None, "the last lease switches it off");
}

/// The first lease to find a warm cache starts the build; a second one waits on it. The first
/// leaving mid-build must not strand the second with an index that never finishes.
#[test]
fn a_build_survives_the_lease_that_started_it_while_another_lease_waits() {
    let mut h = Harness::with_objects([on("n1", "x", "a", "1"), on("n1", "x", "b", "1")]);
    let _table = h.subscribe(all(pods()));
    let first = h.store.lease_node_loads();
    let second = h.store.lease_node_loads();
    drop(first);
    h.settle();
    assert_eq!(
        second.demand("n1"),
        Some(demand(2)),
        "the build finished for the lease that is left"
    );
    drop(second);
}

/// A feed the budget degraded to metadata holds partial resources, which are never placed on a
/// node: the index would read ready and empty. It must read unknown instead, so the Pods column
/// shows the placeholder and the panel lists the node's pods by field selector.
#[test]
fn a_pod_feed_degraded_to_metadata_does_not_answer_with_an_empty_index() {
    use crate::store::{Admission, FeedBudget, FeedKind, FeedRequest};

    struct Degrade;
    impl FeedBudget for Degrade {
        fn admit(&self, request: &FeedRequest, _running: usize) -> Admission {
            if request.kind == FeedKind::Full {
                Admission::Degraded(FeedKind::Metadata)
            } else {
                Admission::Granted
            }
        }
        fn released(&self, _request: &FeedRequest) {}
    }

    let mut h = Harness::with_options(StoreOptions {
        budget: Arc::new(Degrade),
        ..options_with_grace(0)
    });
    h.resources.insert(on("n1", "x", "a", "1"));
    let lease = h.store.lease_node_loads();
    h.settle();
    let key = crate::store::FeedKey::new(pods(), crate::store::FeedScope::Cluster);
    assert_eq!(
        h.store.inner().with_entry(&key, |st| st.request.kind),
        Some(FeedKind::Metadata),
        "the feed was degraded"
    );
    assert!(!lease.is_ready());
    assert_eq!(lease.demand("n1"), None, "unknown, not zero");
    assert_eq!(lease.pods("n1"), None);
    assert_eq!(lease.load("n1"), None);
    assert_eq!(lease.epoch(), 0);
}

#[test]
fn a_namespaced_pod_feed_does_not_feed_the_index() {
    let mut h = Harness::with_objects([on("n1", "x", "a", "1")]);
    let _namespaced = h.subscribe(in_namespaces(pods(), &["x"]));
    let lease = h.store.lease_node_loads();
    // The lease opened the cluster-wide feed of its own; the namespaced one is not consulted.
    assert_eq!(lease.demand("n1"), None, "not listed yet");
    h.settle();
    assert_eq!(lease.demand("n1"), Some(demand(1)));
}

#[test]
fn an_update_at_the_same_version_changes_nothing() {
    let mut index = PodsByNode::default();
    let a = std::sync::Arc::new(StoreObject::Resource(on("n1", "x", "a", "1")));
    index.upsert(&a.key(), &a);
    index.upsert(&a.key(), &a);
    assert_eq!(index.len(), 1);
    assert_eq!(index.demand("n1"), demand(1));
    index.remove(&a.key());
    index.remove(&a.key());
    assert_eq!(index.demand("n1"), Demand::ZERO);
}

/// The per-delta cost of the index at the reference load (10 000 pods on 100 nodes): an update
/// of one pod must not depend on how many pods the index holds. Compared as a ratio against a
/// ten times smaller index, so it holds on a slow machine too.
#[test]
fn a_delta_costs_the_same_at_ten_thousand_pods_as_at_a_thousand() {
    fn per_update(pods: usize) -> f64 {
        let mut index = PodsByNode::default();
        let objects: Vec<_> = (0..pods)
            .map(|i| {
                std::sync::Arc::new(StoreObject::Resource(on(
                    &format!("node-{}", i % 100),
                    "x",
                    &format!("pod-{i}"),
                    "1",
                )))
            })
            .collect();
        for o in &objects {
            index.upsert(&o.key(), o);
        }
        let updates = 20_000;
        let started = Instant::now();
        for i in 0..updates {
            let o = &objects[(i * 7919) % pods];
            index.upsert(&o.key(), o);
        }
        started.elapsed().as_nanos() as f64 / updates as f64
    }
    let small = per_update(1_000);
    let large = per_update(10_000);
    eprintln!("pods-by-node index: {small:.0} ns/update at 1k pods, {large:.0} ns at 10k pods");
    assert!(
        large < small * 4.0 + 2_000.0,
        "an update must not grow with the index: {small:.0} ns at 1k, {large:.0} ns at 10k"
    );
}

#[test]
fn the_node_tables_pods_column_reads_the_index_through_bind() {
    use crate::columns::{ColumnId, ColumnProvider};
    use crate::nodes::NodeColumns;
    use oxikube_testkit::node;

    let mut h = Harness::with_objects([
        on("worker-1", "x", "a", "1"),
        on("worker-1", "x", "b", "1"),
        on("elsewhere", "x", "c", "1"),
    ]);
    let columns = NodeColumns::new();
    let worker = StoreObject::Resource(node().build());
    let cell = |columns: &NodeColumns| {
        columns
            .cell(&worker, &ColumnId::new("pods"), jiff::Timestamp::UNIX_EPOCH)
            .display()
            .to_owned()
    };
    assert_eq!(cell(&columns), "–", "no store bound");
    columns.bind(&h.store);
    assert_eq!(cell(&columns), "–", "the feed has not listed yet");
    h.settle();
    assert_eq!(cell(&columns), "2");
}

fn stored(node: &str, name: &str, rv: &str) -> std::sync::Arc<StoreObject> {
    std::sync::Arc::new(StoreObject::Resource(on(node, "x", name, rv)))
}

fn add(cache: &mut crate::store::cache::ObjectCache, object: &std::sync::Arc<StoreObject>) {
    cache.upsert(object.key(), object.clone());
}

/// Deltas that land while the index is being built from its snapshot are not lost: the pod that
/// was added, the one that moved and the one that was deleted all end up as the cache has them.
#[test]
fn changes_during_the_build_are_replayed_onto_the_result() {
    use crate::store::cache::ObjectCache;
    let (a, b, c) = (
        stored("n1", "a", "1"),
        stored("n1", "b", "1"),
        stored("n2", "c", "1"),
    );
    let mut cache = ObjectCache::default();
    for pod in [&a, &b, &c] {
        add(&mut cache, pod);
    }
    let build = cache
        .track_nodes()
        .expect("a warm cache is built from a snapshot");
    assert!(cache.pods_by_node().is_none(), "not answering yet");

    // While the build runs: d arrives, b moves to n2, c is deleted.
    add(&mut cache, &stored("n2", "d", "1"));
    add(&mut cache, &stored("n2", "b", "2"));
    cache.remove(&c.key());

    let built = PodsByNode::build(build.objects.iter());
    cache.finish_nodes(build.generation, built);
    let index = cache.pods_by_node().expect("installed");
    assert_eq!(index.demand("n1"), demand(1), "a");
    assert_eq!(index.demand("n2"), demand(2), "b (moved) and d; c is gone");
    assert_eq!(index.len(), 3);

    // From here on it follows deltas directly.
    add(&mut cache, &stored("n1", "e", "1"));
    assert_eq!(cache.pods_by_node().unwrap().demand("n1"), demand(2));
}

#[test]
fn a_build_that_outlived_its_claim_is_dropped() {
    use crate::store::cache::ObjectCache;
    let mut cache = ObjectCache::default();
    add(&mut cache, &stored("n1", "a", "1"));
    let old = cache.track_nodes().unwrap();
    cache.untrack_nodes();
    assert!(cache.pods_by_node().is_none());
    // A second claim starts a newer build; the first one's result must not be installed over it.
    let newer = cache.track_nodes().unwrap();
    assert_ne!(old.generation, newer.generation);
    cache.finish_nodes(old.generation, PodsByNode::default());
    assert!(
        cache.pods_by_node().is_none(),
        "the stale result is ignored"
    );
    cache.finish_nodes(newer.generation, PodsByNode::build(newer.objects.iter()));
    assert_eq!(cache.pods_by_node().unwrap().demand("n1"), demand(1));
    cache.untrack_nodes();
    cache.finish_nodes(newer.generation, PodsByNode::default());
    assert!(cache.pods_by_node().is_none(), "nobody wants it any more");
}

#[test]
fn an_empty_cache_has_the_index_at_once() {
    use crate::store::cache::ObjectCache;
    let mut cache = ObjectCache::default();
    assert!(cache.track_nodes().is_none(), "nothing to build from");
    assert!(cache.pods_by_node().is_some());
}

/// The index's version moves with a node's load (a pod arriving, leaving, moving or asking for
/// other amounts) and not with a pod's status: the lease's epoch is built on it.
#[test]
fn the_index_version_moves_with_a_nodes_load_and_not_with_a_status_change() {
    let mut index = PodsByNode::default();
    let v0 = index.version();
    let a = stored("n1", "a", "1");
    index.upsert(&a.key(), &a);
    let v1 = index.version();
    assert_ne!(v1, v0, "a pod arrived");
    // Same node, same requests, a newer version of the pod (its status changed).
    let a2 = stored("n1", "a", "2");
    index.upsert(&a2.key(), &a2);
    assert_eq!(index.version(), v1, "no node's load moved");
    assert_eq!(index.demand("n1"), demand(1));
    assert!(
        std::sync::Arc::ptr_eq(&index.pods("n1")[0], &a2),
        "but the index holds the newer object"
    );
    let moved = stored("n2", "a", "3");
    index.upsert(&moved.key(), &moved);
    assert_ne!(index.version(), v1, "the pod moved to another node");
    let v2 = index.version();
    index.remove(&moved.key());
    assert_ne!(index.version(), v2, "and left");
}
