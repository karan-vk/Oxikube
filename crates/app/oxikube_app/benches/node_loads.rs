//! Micro-benchmark (E12-S09): what the pods-by-node index adds to a pod delta.
//!
//! The same 500-event churn (400 modifies, 50 adds, 50 deletes, a tenth of the modified pods
//! moving to another node) is applied to a 10 000-pod cache on 100 nodes twice: with only a
//! subscriber that keeps no rows (what a `CountsLease` does), and with a `NodeLoadsLease` that
//! also switches the index on. The difference per delta is the index's cost.
//!
//! A second measurement times what the UI thread pays to take the lease on a warm feed (the Pods
//! table or a sidebar count already holds the 10 000 pods): the `lease_node_loads` call itself,
//! against the build the store runs on its spawner.
//!
//! `cargo bench -p oxikube_app --bench node_loads` prints the median batch time of each and the
//! cost per delta. Under `cargo test --all-targets` (no `--bench` flag) it runs one small
//! iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::task::LocalSpawnExt;
use oxikube_app::store::{
    ResourceStore, StoreFilter, StoreOptions, StorePorts, StoreQuery, StoreRuntime,
};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::session::WatchScope;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort, Timeline, pod};
use parking_lot::Mutex;

const OBJECTS: usize = 10_000;
const NODES: usize = 100;
const BATCH: usize = 500;

fn object(i: usize, node: usize, rv: usize) -> Resource {
    let mut r = pod()
        .namespace(format!("ns-{}", i % 20))
        .name(format!("pod-{i:05}"))
        .node(format!("node-{}", node % NODES))
        .requests("100m", "128Mi")
        .limits("200m", "256Mi")
        .build();
    r.meta.resource_version = Some(rv.to_string().into());
    r
}

fn churn(round: usize, rv: &mut usize) -> DeltaBatch<Resource> {
    let mut deltas = Vec::with_capacity(BATCH);
    let base = round * BATCH;
    for k in 0..BATCH {
        *rv += 1;
        let i = (base + k * 17) % OBJECTS;
        deltas.push(match k % 10 {
            0 => Delta::Applied(object(OBJECTS + base + k, k, *rv)),
            1 => Delta::Deleted(object(i, i, *rv)),
            // Every tenth modify moves the pod to the next node.
            5 => Delta::Applied(object(i, i + 1, *rv)),
            _ => Delta::Applied(object(i, i, *rv)),
        });
    }
    DeltaBatch::from_deltas(deltas)
}

/// The median time to apply a churn batch, with the index on or off.
fn measure(indexed: bool, warmup: usize, samples: usize) -> Duration {
    let clock = Arc::new(FakeClockPort::default());
    let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
    let mut rv = 0;
    let initial: Vec<Resource> = (0..OBJECTS)
        .map(|i| {
            rv += 1;
            object(i, i, rv)
        })
        .collect();
    let mut timeline = Timeline::new().ok_at(
        Duration::ZERO,
        DeltaBatch::from_deltas(vec![Delta::Restarted(initial)]),
    );
    for round in 0..warmup + samples {
        timeline = timeline.ok_at(Duration::from_secs(round as u64 + 1), churn(round, &mut rv));
    }
    resources.script().watch.push_ok(timeline.keep_open());

    let queue: Arc<Mutex<Vec<BoxFuture<'static, ()>>>> = Arc::default();
    let spawn_queue = queue.clone();
    let store = ResourceStore::new(
        ClusterId::new("bench", &ContextName::from("bench")),
        StorePorts {
            resources,
            tables: Arc::new(FakeTableFeedPort::with_clock(clock.clone())),
        },
        StoreRuntime {
            spawner: Arc::new(move |task| spawn_queue.lock().push(task)),
            clock: clock.clone(),
            probe: None,
        },
        StoreOptions::default(),
    );
    let mut pool = LocalPool::new();
    let run = |pool: &mut LocalPool| loop {
        for task in std::mem::take(&mut *queue.lock()) {
            pool.spawner().spawn_local(task).expect("spawn");
        }
        pool.run_until_stalled();
        if queue.lock().is_empty() {
            break;
        }
    };

    // The lease is the index; the plain subscription keeps the feed open and no rows.
    let lease = indexed.then(|| store.lease_node_loads());
    let _no_rows = (!indexed).then(|| {
        store.subscribe(
            StoreQuery::new(Gvk::new("", "v1", "Pod"), WatchScope::Cluster).with_filter(
                StoreFilter {
                    name: Some(String::new()),
                    ..StoreFilter::default()
                },
            ),
        )
    });
    run(&mut pool);
    if let Some(lease) = &lease {
        assert_eq!(
            lease.demand("node-0").map(|d| d.pods),
            Some(OBJECTS as u32 / NODES as u32),
            "the index listed the pods"
        );
    }

    let mut times = Vec::with_capacity(samples);
    for round in 0..warmup + samples {
        clock.advance(Duration::from_secs(1));
        let start = Instant::now();
        run(&mut pool);
        let elapsed = start.elapsed();
        if round >= warmup {
            times.push(elapsed);
        }
    }
    times.sort();
    times[times.len() / 2]
}

/// `(the lease call on the caller's thread, the index build on the spawner)` for a lease taken on
/// a feed that already holds [`OBJECTS`] pods.
fn measure_warm_lease() -> (Duration, Duration) {
    let clock = Arc::new(FakeClockPort::default());
    let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
    let mut rv = 0;
    let initial: Vec<Resource> = (0..OBJECTS)
        .map(|i| {
            rv += 1;
            object(i, i, rv)
        })
        .collect();
    resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::ZERO,
                DeltaBatch::from_deltas(vec![Delta::Restarted(initial)]),
            )
            .keep_open(),
    );
    let queue: Arc<Mutex<Vec<BoxFuture<'static, ()>>>> = Arc::default();
    let spawn_queue = queue.clone();
    let store = ResourceStore::new(
        ClusterId::new("bench", &ContextName::from("bench")),
        StorePorts {
            resources,
            tables: Arc::new(FakeTableFeedPort::with_clock(clock.clone())),
        },
        StoreRuntime {
            spawner: Arc::new(move |task| spawn_queue.lock().push(task)),
            clock,
            probe: None,
        },
        StoreOptions::default(),
    );
    let mut pool = LocalPool::new();
    let run = |pool: &mut LocalPool| loop {
        for task in std::mem::take(&mut *queue.lock()) {
            pool.spawner().spawn_local(task).expect("spawn");
        }
        pool.run_until_stalled();
        if queue.lock().is_empty() {
            break;
        }
    };
    let _no_rows = store.subscribe(
        StoreQuery::new(Gvk::new("", "v1", "Pod"), WatchScope::Cluster).with_filter(StoreFilter {
            name: Some(String::new()),
            ..StoreFilter::default()
        }),
    );
    run(&mut pool);

    let started = Instant::now();
    let lease = store.lease_node_loads();
    let call = started.elapsed();
    let started = Instant::now();
    run(&mut pool);
    let build = started.elapsed();
    assert_eq!(
        lease.demand("node-0").map(|d| d.pods),
        Some(OBJECTS as u32 / NODES as u32),
        "the index covers the cached pods"
    );
    (call, build)
}

fn main() {
    let bench = std::env::args().any(|a| a == "--bench");
    let (warmup, samples) = if bench { (5, 60) } else { (0, 1) };
    let without = measure(false, warmup, samples);
    let with = measure(true, warmup, samples);
    let (call, build) = measure_warm_lease();
    if bench {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let extra = with.saturating_sub(without);
        println!(
            "node_loads: {BATCH}-event batch into {OBJECTS} pods on {NODES} nodes, median: \
             {:.3} ms without the index, {:.3} ms with it",
            ms(without),
            ms(with)
        );
        println!(
            "node_loads: the index adds {:.3} ms per batch, {:.0} ns per delta",
            ms(extra),
            extra.as_nanos() as f64 / BATCH as f64
        );
        println!(
            "node_loads: taking the lease on a feed with {OBJECTS} pods costs the caller \
             {:.3} ms; the index is built on the spawner in {:.3} ms",
            ms(call),
            ms(build)
        );
    }
}
