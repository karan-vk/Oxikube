//! Micro-benchmark (E07-S04): typing a filter over a warm 10 000-object feed.
//!
//! A view that holds all 10k rows types `pod-0123` one character at a time (each step narrows
//! the rows it already has, the worst case being the steps that keep nearly all of them), then
//! deletes it back (each step recomputes from the cache), then applies a regex and a fuzzy
//! filter. Each step is timed in two parts: the caller's side (`set_filter_parts`, what the UI
//! thread pays) and the spawner's side (the filter pass up to the snapshot being ready).
//!
//! `cargo bench -p oxikube_app --bench store_filter` prints min / median / p95 / max. Under
//! `cargo test --all-targets` it runs one small iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::task::LocalSpawnExt;
use futures::{FutureExt, StreamExt};
use oxikube_app::store::filter::parse;
use oxikube_app::store::{
    ResourceStore, StoreOptions, StorePorts, StoreQuery, StoreRuntime, Subscription,
};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::session::WatchScope;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort, Timeline, pod};
use parking_lot::Mutex;

const OBJECTS: usize = 10_000;

fn object(i: usize) -> Resource {
    let mut r = pod()
        .namespace(format!("ns-{}", i % 20))
        .name(format!("pod-{i:05}"))
        .label("app", format!("app-{}", i % 50))
        .build();
    r.meta.resource_version = Some("1".into());
    r
}

fn main() {
    let bench = std::env::args().any(|a| a == "--bench");
    let samples = if bench { 40 } else { 1 };

    let clock = Arc::new(FakeClockPort::default());
    let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
    let initial: Vec<Resource> = (0..OBJECTS).map(object).collect();
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
    // Keeps the feed warm for the whole run.
    let mut anchor = store.subscribe(StoreQuery::new(pods.clone(), WatchScope::Cluster));
    run(&mut pool);
    assert_eq!(
        anchor.next().now_or_never().flatten().map(|d| d.len),
        Some(OBJECTS)
    );

    let typing = [
        "p", "po", "pod", "pod-", "pod-0", "pod-01", "pod-012", "pod-0123",
    ];
    let deleting = ["pod-012", "pod-01", "pod-0", "pod-", "pod", "po", "p", ""];
    let mut narrowing = Vec::new();
    let mut recompute = Vec::new();
    let mut regex = Vec::new();
    let mut fuzzy = Vec::new();
    let mut caller = Vec::new();

    let mut step = |view: &mut Subscription, input: &str, pool: &mut LocalPool| {
        let parts = parse(input).expect("parses").parts();
        let sort = parts.sort(None);
        let start = Instant::now();
        view.set_filter_parts(parts, sort);
        let on_caller = start.elapsed();
        let start = Instant::now();
        run(pool);
        let on_spawner = start.elapsed();
        let delta = view.next().now_or_never().flatten();
        assert!(delta.is_some(), "{input:?} delivered");
        caller.push(on_caller);
        on_spawner
    };

    for _ in 0..samples + usize::from(bench) * 3 {
        let mut view = store.subscribe(StoreQuery::new(pods.clone(), WatchScope::Cluster));
        run(&mut pool);
        assert_eq!(
            view.next().now_or_never().flatten().map(|d| d.len),
            Some(OBJECTS)
        );
        for input in typing {
            narrowing.push(step(&mut view, input, &mut pool));
        }
        for input in deleting {
            recompute.push(step(&mut view, input, &mut pool));
        }
        regex.push(step(&mut view, "^pod-0[0-4].*[0-9]$", &mut pool));
        fuzzy.push(step(&mut view, "-f pd1", &mut pool));
        drop(view);
        run(&mut pool);
    }

    if bench {
        report(
            "filter: type a char (narrow the held rows), spawner side",
            narrowing,
        );
        report(
            "filter: delete a char (recompute from the cache), spawner side",
            recompute,
        );
        report("filter: regex over 10k, spawner side", regex);
        report("filter: fuzzy over 10k, spawner side", fuzzy);
        report("filter: every step, caller (UI thread) side", caller);
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
