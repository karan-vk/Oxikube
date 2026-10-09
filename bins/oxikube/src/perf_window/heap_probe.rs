//! What a connected cluster's pods cost on the heap, measured through the app's real services,
//! stores and tables (E07-P603).
//!
//! The `idle` scenario's two clusters of 1 000 pods are run on GPUI's test platform under a
//! counting global allocator (installed for the bin's tests only, counting the calling thread's
//! allocations: GPUI's test platform runs the whole app on the test's thread, and the other tests of
//! this binary run beside it on theirs), once with a small cluster and
//! once with a large one; the difference in live heap bytes, divided by the difference in pods, is
//! the cost of one pod across everything that holds it: the cluster's feed cache, the sorted
//! indices, the table rows, the sidebar's counts. No window, no GPU and no `vmmap` needed, so it
//! runs anywhere, and it counts bytes exactly instead of sampling RSS.
//!
//! ```text
//! OXIKUBE_HEAP_PODS=1000 cargo test -p oxikube --features perf-window heap_probe -- --ignored --nocapture
//! ```
//!
//! The asserting test below is not ignored: it runs the two sizes in one process and fails when a
//! pod costs more than [`BUDGET_PER_POD`] bytes of live heap.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use gpui::TestAppContext;

use super::Scenario;
use super::tests::run_in;
use super::world::{ClusterSpec, WorldSpec};

struct Counting;

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
}

fn add(bytes: isize) {
    // `try_with`: the allocator also runs while a thread's locals are being torn down.
    let _ = LIVE.try_with(|live| live.set(live.get() + bytes));
}

fn live() -> isize {
    LIVE.with(Cell::get)
}

// SAFETY: forwards every call to the system allocator unchanged and only updates a counter.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        add(layout.size() as isize);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        add(-(layout.size() as isize));
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        add(new_size as isize - layout.size() as isize);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Live heap bytes per pod the `idle` scenario's views and stores may hold. Held as `Value` trees a
/// pod cost 10 400 here (and 17 000 in the issue's release measurement, which counts more of the
/// app); compact it costs about 2 200, of which about a thousand is the synthetic server's own copy.
/// This is the "at least half" of E07-P603 with room for the allocator's noise.
const BUDGET_PER_POD: f64 = 5_000.0;

fn world(pods: usize) -> WorldSpec {
    WorldSpec {
        clusters: vec![
            ClusterSpec::still(super::MAIN_CONTEXT, pods),
            ClusterSpec::still("perf-b", pods),
        ],
        listed_only: 0,
    }
}

/// Live heap bytes the idle scenario added over two clusters of `pods` pods each (its app, window,
/// stores and tables stay alive in `cx` until the test ends).
fn added_by_idle(cx: &mut TestAppContext, pods: usize) -> isize {
    let before = live();
    run_in(cx, Scenario::Idle, &world(pods)).expect("the idle scenario ran");
    live() - before
}

fn pods_from_env() -> usize {
    std::env::var("OXIKUBE_HEAP_PODS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000)
}

/// Prints the live heap the idle scenario added at `OXIKUBE_HEAP_PODS` pods per cluster.
#[gpui::test]
#[ignore = "a measurement: run with --ignored --nocapture"]
fn heap_probe_prints_added_bytes(cx: &mut TestAppContext) {
    let pods = pods_from_env();
    let added = added_by_idle(cx, pods);
    eprintln!("heap_probe: {pods} pods per cluster added {added} live heap bytes");
}

/// The first app of a process pays one-time costs (font and theme caches), so the large world
/// runs first and carries them: the figure can only come out too high, and the budget is a ceiling.
#[gpui::test]
fn heap_probe_a_pod_costs_a_fraction_of_a_value_tree(
    large: &mut TestAppContext,
    small: &mut TestAppContext,
) {
    let (few, many) = (20, 270);
    let large_bytes = added_by_idle(large, many);
    let small_bytes = added_by_idle(small, few);
    let per_pod = (large_bytes - small_bytes) as f64 / (2.0 * (many - few) as f64);
    eprintln!("heap_probe: {per_pod:.0} live heap bytes per pod across both clusters");
    assert!(
        per_pod < BUDGET_PER_POD,
        "a pod costs {per_pod:.0} live heap bytes in the idle scenario (budget {BUDGET_PER_POD})"
    );
}
