//! Micro-benchmark (E14-S02): the `EventService` buffer at its default bound of 10 000 events.
//!
//! The acceptance budgets are "insert 10 000 events in under 50 ms" and "a 10k buffer under
//! 50 MB" (epic E14, done-when 5). This times the inserts into an empty buffer, a relist into a
//! new buffer (what the feed task builds outside the lock), a storm of updates to held events (a
//! crash-looping pod re-reporting `BackOff`), the queries a view and a badge make, and counts the
//! bytes the full buffer holds with a counting allocator: the events themselves (realistic
//! messages and names) plus the ring and every index.
//!
//! `cargo bench -p oxikube_app --bench event_buffer` prints the numbers (best of 5 runs). Under
//! `cargo test --all-targets` (no `--bench` flag) it runs one small iteration.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use jiff::{SignedDuration, Timestamp};
use oxikube_app::events::{EventBuffer, EventFilter};
use oxikube_domain::event::{Event, EventType};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};

/// Counts the bytes live on the heap.
struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

// SAFETY: forwards to the system allocator and only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: the caller's layout contract is passed through unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: `ptr` came from `alloc` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn live() -> isize {
    LIVE.load(Ordering::Relaxed)
}

/// `n` distinct events as a busy cluster writes them: 50 namespaces, one object per 5 events,
/// one in five a `Warning`, messages of 80 to 200 bytes.
fn events(n: usize) -> Vec<Event> {
    let cluster = ClusterId::new("/home/me/.kube/config", &ContextName::new("prod"));
    let pod = Gvk::new("", "v1", "Pod");
    let t0: Timestamp = "2026-10-01T00:00:00Z".parse().expect("a timestamp");
    (0..n)
        .map(|i| {
            let ns: Arc<str> = Arc::from(format!("team-{:02}", i % 50));
            let name = format!("checkout-api-7d9f8c6b5-{:05}", i / 5);
            let warning = i % 5 == 0;
            let at = t0 + SignedDuration::from_secs(i as i64);
            Event {
                uid: Some(Arc::from(format!("2f7c1d3e-5a6b-4c8d-9e0f-{i:012}"))),
                namespace: Some(ns.clone()),
                event_type: if warning { EventType::Warning } else { EventType::Normal },
                reason: Arc::from(if warning { "BackOff" } else { "Pulled" }),
                message: if warning {
                    format!("Back-off restarting failed container app in pod {name}_{ns}({i:08x}-aaaa-bbbb-cccc-dddddddddddd)")
                } else {
                    format!("Container image \"registry.example.com/shop/checkout-api:1.{}.{}\" already present on machine", i % 40, i % 7)
                },
                truncated: false,
                regarding: ResourceRef::new(cluster.clone(), pod.clone(), Some(ns), name.as_str()),
                regarding_uid: Some(Arc::from(format!("9a8b7c6d-1e2f-3a4b-5c6d-{:012}", i / 5))),
                related: None,
                count: 1,
                first_seen: Some(at),
                last_seen: Some(at),
                reporting_component: Some(Arc::from("kubelet")),
                reporting_instance: Some(Arc::from(format!("node-{:03}", i % 100))),
                action: None,
            }
        })
        .collect()
}

fn best<T>(runs: usize, mut f: impl FnMut() -> (Duration, T)) -> (Duration, T) {
    let mut out = f();
    for _ in 1..runs {
        let next = f();
        if next.0 < out.0 {
            out = next;
        }
    }
    out
}

fn run(n: usize, runs: usize) {
    let source = events(n);

    let (insert, buffer) = best(runs, || {
        let batch = source.clone();
        let mut buffer = EventBuffer::new(n);
        let started = Instant::now();
        for event in batch {
            buffer.upsert(event);
        }
        (started.elapsed(), buffer)
    });
    assert_eq!(
        buffer.len(),
        n,
        "every distinct event fits: the buffer is sized to the batch"
    );
    drop(buffer);

    let (relist, _) = best(runs, || {
        let batch = source.clone();
        let started = Instant::now();
        let buffer = EventBuffer::from_events(n, batch);
        (started.elapsed(), buffer.len())
    });

    // Memory: what a full buffer holds, events included: counted from before the events are
    // made, after the list that carried them is gone.
    let before = live();
    let batch = events(n);
    let mut buffer = EventBuffer::new(n);
    for event in batch {
        buffer.upsert(event);
    }
    let held = (live() - before).max(0) as f64;
    let per_event = held / n as f64;

    // A storm of updates: every warning re-reported with a higher count and a later time.
    let t_end = Timestamp::from_second(1_900_000_000).expect("a timestamp");
    let updates: Vec<Event> = source
        .iter()
        .filter(|e| e.is_warning())
        .cloned()
        .map(|mut e| {
            e.count += 1;
            e.last_seen = Some(t_end);
            e
        })
        .collect();
    let updated = updates.len();
    let started = Instant::now();
    for event in updates {
        buffer.upsert(event);
    }
    let update = started.elapsed();
    assert_eq!(
        buffer.len(),
        n,
        "updates must upsert in place, not add rows"
    );

    // What views read: the first screen of the events view, the faults-only screen, one
    // object's tab, and a badge per visible row.
    let all = EventFilter::new();
    let faults = EventFilter::warnings();
    let object = EventFilter::new().object(source[n / 2].regarding_uid.clone().expect("a uid"));
    let started = Instant::now();
    let shown = buffer.query(&all).take(60).count() + buffer.query(&faults).take(60).count();
    let screens = started.elapsed();
    let started = Instant::now();
    let tab = buffer.query(&object).count();
    let tab_time = started.elapsed();
    let started = Instant::now();
    let badges: usize = source
        .iter()
        .take(60)
        .map(|e| {
            buffer
                .object_counts(e.regarding_uid.as_deref().unwrap_or(""))
                .1
        })
        .sum();
    let badge_time = started.elapsed();

    println!(
        "{n} inserts: {insert:.2?} (budget 50 ms){} | relist into a new buffer: {relist:.2?} | {updated} updates: {update:.2?}",
        if insert < Duration::from_millis(50) {
            ""
        } else {
            "  OVER BUDGET"
        },
    );
    // The budget is 50 MB for 10 000 events; larger bounds scale it.
    let budget = 50.0 * 1_048_576.0 * (n as f64 / 10_000.0).max(1.0);
    println!(
        "  memory of the full buffer: {:.1} MB ({per_event:.0} bytes/event, budget {:.0} MB){}",
        held / 1_048_576.0,
        budget / 1_048_576.0,
        if held < budget { "" } else { "  OVER BUDGET" },
    );
    println!(
        "  two 60-row screens: {screens:.1?} ({shown} rows) | one object's tab: {tab_time:.1?} ({tab} events) | 60 badges: {badge_time:.1?} ({badges} warnings)"
    );
    assert!(held < budget, "a full buffer is over budget");
}

fn main() {
    if std::env::args().any(|a| a == "--bench") {
        run(10_000, 5);
        run(50_000, 3);
    } else {
        run(1_000, 1);
    }
}
