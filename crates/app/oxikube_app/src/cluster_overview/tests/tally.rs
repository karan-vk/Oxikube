//! The tallies: health and allocation from pods and nodes, and the incremental tally matching a
//! full recompute after any sequence of adds, phase changes and deletes (property test).

use std::collections::BTreeMap;

use oxikube_domain::{Demand, Resource};
use oxikube_testkit::{node, pod};
use proptest::prelude::*;

use crate::cluster_overview::{Capacity, NodeCounts, NodeTally, PhaseCounts, PodTally};
use crate::store::ObjectKey;

fn key(r: &Resource) -> ObjectKey {
    ObjectKey::of(&r.meta)
}

#[test]
fn pods_count_by_phase_and_only_scheduled_live_pods_hold_resources() {
    let pods = [
        pod()
            .namespace("a")
            .name("run")
            .running()
            .node("n1")
            .requests("250m", "128Mi")
            .build(),
        pod()
            .namespace("a")
            .name("wait")
            .pending()
            .requests("1", "1Gi")
            .build(),
        pod()
            .namespace("b")
            .name("done")
            .succeeded()
            .node("n1")
            .requests("2", "2Gi")
            .build(),
        pod().namespace("b").name("bad").failed().node("n2").build(),
    ];
    let tally = PodTally::of(pods.iter());
    assert_eq!(
        tally.phases(),
        PhaseCounts {
            running: 1,
            pending: 1,
            succeeded: 1,
            failed: 1,
            unknown: 0,
        }
    );
    assert_eq!(tally.phases().total(), 4);
    let demand = tally.demand();
    assert_eq!(demand.pods, 1, "only the running pod on a node counts");
    assert_eq!(demand.cpu_requests, 250);
    assert_eq!(demand.memory_requests, 128 * 1024 * 1024);
}

#[test]
fn a_phase_change_and_a_delete_move_the_counts() {
    let mut tally = PodTally::default();
    let p = pod().namespace("a").name("web").pending().build();
    tally.upsert(key(&p), &p);
    assert_eq!(tally.phases().pending, 1);
    let running = pod()
        .namespace("a")
        .name("web")
        .running()
        .node("n1")
        .requests("100m", "64Mi")
        .build();
    tally.upsert(key(&running), &running);
    assert_eq!(tally.phases().pending, 0);
    assert_eq!(tally.phases().running, 1);
    assert_eq!(tally.demand().cpu_requests, 100);
    tally.remove(&key(&running));
    assert_eq!(tally.phases(), PhaseCounts::default());
    assert_eq!(tally.demand(), Demand::ZERO);
    assert!(tally.is_empty());
}

#[test]
fn nodes_count_by_readiness_and_sum_what_they_allocate() {
    let nodes = [
        node().name("a").allocatable("4", "16Gi", "110").build(),
        node()
            .name("b")
            .allocatable("2", "8Gi", "110")
            .cordoned()
            .build(),
        node()
            .name("c")
            .not_ready()
            .allocatable("8", "32Gi", "250")
            .build(),
    ];
    let mut tally = NodeTally::of(nodes.iter());
    assert_eq!(
        tally.counts(),
        NodeCounts {
            ready: 2,
            not_ready: 1,
            unknown: 0,
            cordoned: 1,
        }
    );
    assert_eq!(
        tally.capacity(),
        Capacity {
            cpu_millis: 14_000,
            memory_bytes: 56 * 1024 * 1024 * 1024,
            pods: 470,
        }
    );
    tally.remove("c");
    assert_eq!(tally.counts().total(), 2);
    assert_eq!(tally.counts().unhealthy(), 0);
    assert_eq!(tally.capacity().cpu_millis, 6_000);
    // A node turning NotReady moves between the counts, its capacity stays.
    tally.upsert(
        &node()
            .name("a")
            .not_ready()
            .allocatable("4", "16Gi", "110")
            .build(),
    );
    assert_eq!(tally.counts().ready, 1);
    assert_eq!(tally.counts().not_ready, 1);
    assert_eq!(tally.capacity().cpu_millis, 6_000);
}

/// One step of a churn script: a pod `slot` takes a state, or is deleted.
#[derive(Debug, Clone)]
enum Step {
    Set {
        slot: u8,
        phase: u8,
        node: u8,
        cpu: u16,
    },
    Delete {
        slot: u8,
    },
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => (0u8..12, 0u8..5, 0u8..4, 0u16..2000)
            .prop_map(|(slot, phase, node, cpu)| Step::Set { slot, phase, node, cpu }),
        1 => (0u8..12).prop_map(|slot| Step::Delete { slot }),
    ]
}

fn pod_of(slot: u8, phase: u8, node: u8, cpu: u16) -> Resource {
    let builder = pod()
        .namespace(format!("ns-{}", slot % 3))
        .name(format!("p-{slot}"))
        .requests(format!("{cpu}m"), "64Mi");
    let builder = match phase {
        0 => builder.pending(),
        1 => builder.running(),
        2 => builder.succeeded(),
        3 => builder.failed(),
        _ => builder,
    };
    // Node 0 means "not scheduled".
    let builder = if node == 0 {
        builder
    } else {
        builder.node(format!("n{node}"))
    };
    builder.build()
}

proptest! {
    #[test]
    fn the_incremental_tally_matches_a_full_recompute(steps in prop::collection::vec(step(), 0..60)) {
        let mut tally = PodTally::default();
        let mut live: BTreeMap<u8, Resource> = BTreeMap::new();
        for step in steps {
            match step {
                Step::Set { slot, phase, node, cpu } => {
                    let pod = pod_of(slot, phase, node, cpu);
                    tally.upsert(key(&pod), &pod);
                    live.insert(slot, pod);
                }
                Step::Delete { slot } => {
                    if let Some(pod) = live.remove(&slot) {
                        tally.remove(&key(&pod));
                    }
                }
            }
            let full = PodTally::of(live.values());
            prop_assert_eq!(tally.phases(), full.phases());
            prop_assert_eq!(tally.demand(), full.demand());
            prop_assert_eq!(tally.len(), live.len());
        }
    }

    #[test]
    fn the_node_tally_matches_a_full_recompute(steps in prop::collection::vec((0u8..6, any::<bool>(), 1u8..8, any::<bool>()), 0..40)) {
        let mut tally = NodeTally::default();
        let mut live: BTreeMap<u8, Resource> = BTreeMap::new();
        for (slot, ready, cpu, delete) in steps {
            if delete {
                if live.remove(&slot).is_some() {
                    tally.remove(&format!("node-{slot}"));
                }
                continue;
            }
            let builder = node().name(format!("node-{slot}")).allocatable(cpu.to_string(), "4Gi", "110");
            let n = if ready { builder.build() } else { builder.not_ready().build() };
            tally.upsert(&n);
            live.insert(slot, n);
        }
        let full = NodeTally::of(live.values());
        prop_assert_eq!(tally.counts(), full.counts());
        prop_assert_eq!(tally.capacity(), full.capacity());
    }
}
