//! Micro-benchmark (E07-S01): apply a 500-event batch to a 10 000-object `ResourceStore` cache
//! with two live subscribers (default order and newest-first), measured from the feed handing
//! over the batch to both subscribers' coalesced `StoreDelta`s being ready.
//!
//! It also times a view joining the warm 10k feed and then changing its filter and sort: the
//! caller's side (what the UI thread pays; it must stay far below a frame) and the seeding task
//! on the spawner (the filter pass and the sort).
//!
//! `cargo bench -p oxikube_app --bench store_apply` prints min / median / p95 / max. Under
//! `cargo test --all-targets` (no `--bench` flag) it runs one small iteration as a smoke test.
//! Criterion-style warm-up and sampling, without the dependency.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::task::LocalSpawnExt;
use futures::{FutureExt, StreamExt};
use oxikube_app::store::{
    ResourceStore, SortField, SortKey, StoreFilter, StoreOptions, StorePorts, StoreQuery,
    StoreRuntime, Subscription,
};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::session::WatchScope;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort, Timeline, pod};
use parking_lot::Mutex;

const OBJECTS: usize = 10_000;
const BATCH: usize = 500;

fn object(i: usize, rv: usize) -> Resource {
    let mut r = pod()
        .namespace(format!("ns-{}", i % 20))
        .name(format!("pod-{i:05}"))
        .label("app", format!("app-{}", i % 50))
        .created(format!(
            "2026-01-01T{:02}:{:02}:{:02}Z",
            (i / 3600) % 24,
            (i / 60) % 60,
            i % 60
        ))
        .restarts((rv % 7) as u32)
        .build();
    r.meta.resource_version = Some(rv.to_string().into());
    r
}

/// 400 modifies, 50 adds, 50 deletes; ids rotate so every batch touches new objects.
fn churn(round: usize, rv: &mut usize) -> DeltaBatch<Resource> {
    let mut deltas = Vec::with_capacity(BATCH);
    let base = round * BATCH;
    for k in 0..BATCH {
        *rv += 1;
        let i = (base + k * 17) % OBJECTS;
        deltas.push(match k % 10 {
            0 => Delta::Applied(object(OBJECTS + base + k, *rv)),
            1 => Delta::Deleted(object(i, *rv)),
            _ => Delta::Applied(object(i, *rv)),
        });
    }
    DeltaBatch::from_deltas(deltas)
}

fn main() {
    let bench = std::env::args().any(|a| a == "--bench");
    let (warmup, samples) = if bench { (5, 60) } else { (0, 1) };

    let clock = Arc::new(FakeClockPort::default());
    let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
    let mut rv = 0;
    let initial: Vec<Resource> = (0..OBJECTS)
        .map(|i| {
            rv += 1;
            object(i, rv)
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

    let pods = Gvk::new("", "v1", "Pod");
    let mut subs: Vec<Subscription> = vec![
        store.subscribe(StoreQuery::new(pods.clone(), WatchScope::Cluster)),
        store.subscribe(
            StoreQuery::new(pods, WatchScope::Cluster)
                .with_sort(SortKey::by(SortField::Created).descending()),
        ),
    ];
    run(&mut pool);
    let drain = |subs: &mut [Subscription]| -> usize {
        subs.iter_mut()
            .filter_map(|s| s.next().now_or_never().flatten())
            .map(|d| d.len)
            .sum()
    };
    assert_eq!(drain(&mut subs), 2 * OBJECTS, "initial list delivered");

    let mut times = Vec::with_capacity(samples);
    for round in 0..warmup + samples {
        clock.advance(Duration::from_secs(1));
        let start = Instant::now();
        run(&mut pool);
        let rows = drain(&mut subs);
        let elapsed = start.elapsed();
        assert!(rows > 0, "round {round} delivered");
        if round >= warmup {
            times.push(elapsed);
        }
    }
    let mut caller = Vec::with_capacity(samples);
    let mut seeding = Vec::with_capacity(samples);
    for round in 0..warmup + samples {
        let start = Instant::now();
        let mut view = store.subscribe(StoreQuery::new(
            Gvk::new("", "v1", "Pod"),
            WatchScope::Cluster,
        ));
        let subscribe = start.elapsed();
        let start = Instant::now();
        run(&mut pool);
        let seed_subscribe = start.elapsed();
        let first = view.next().now_or_never().flatten().map(|d| d.len);
        assert!(first.is_some_and(|len| len > 0), "the view is seeded");

        let start = Instant::now();
        view.set_filter(StoreFilter::text("pod-0"));
        view.set_sort(SortKey::by(SortField::Created).descending());
        let reseed = start.elapsed();
        let start = Instant::now();
        run(&mut pool);
        let seed_reseed = start.elapsed();
        assert!(view.next().now_or_never().flatten().is_some(), "re-seeded");
        drop(view);
        run(&mut pool);
        if round >= warmup {
            caller.push(subscribe.max(reseed));
            seeding.push(seed_subscribe.max(seed_reseed));
        }
    }

    if bench {
        report(
            &format!("store_apply: {BATCH}-event batch into {OBJECTS} objects, 2 subscribers"),
            times,
        );
        report(
            &format!(
                "store_seed: caller side of subscribe / set_filter+set_sort on {OBJECTS} objects"
            ),
            caller,
        );
        report(
            &format!("store_seed: seeding task on the spawner, {OBJECTS} objects"),
            seeding,
        );
    }
}

/// Prints min / median / p95 / max of `times`.
fn report(what: &str, mut times: Vec<Duration>) {
    times.sort();
    let pick = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!(
        "{what}, {} samples: min {:.3} ms, median {:.3} ms, p95 {:.3} ms, max {:.3} ms",
        times.len(),
        ms(times[0]),
        ms(pick(0.5)),
        ms(pick(0.95)),
        ms(times[times.len() - 1]),
    );
}
