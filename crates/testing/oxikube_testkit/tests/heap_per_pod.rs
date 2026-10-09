//! Heap cost of one watched pod (E07-P603): the retained bytes of a [`Resource`] against the
//! `serde_json::Value` tree the same pod used to be held as.
//!
//! This binary installs a counting allocator that tallies live bytes and blocks per thread, so the
//! numbers are exact and need no tool (`heap`, `vmmap`) or window. Run it with
//! `cargo test -p oxikube_testkit --test heap_per_pod -- --nocapture` to see them.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use oxikube_domain::{JsonDoc, Resource};
use oxikube_testkit::pod;

struct Counting;

thread_local! {
    static BYTES: Cell<isize> = const { Cell::new(0) };
    static BLOCKS: Cell<isize> = const { Cell::new(0) };
}

fn add(bytes: isize, blocks: isize) {
    // `try_with`: the allocator also runs while a thread's locals are being torn down.
    let _ = BYTES.try_with(|b| b.set(b.get() + bytes));
    let _ = BLOCKS.try_with(|b| b.set(b.get() + blocks));
}

// SAFETY: forwards every call to the system allocator unchanged and only updates counters.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        add(layout.size() as isize, 1);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        add(-(layout.size() as isize), -1);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        add(new_size as isize - layout.size() as isize, 0);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn live() -> (isize, isize) {
    (BYTES.with(Cell::get), BLOCKS.with(Cell::get))
}

/// Pods the way the windowed scenarios and `cargo xtask load-pods` shape them.
fn build(i: usize) -> Resource {
    pod()
        .namespace(format!("oxikube-load-{}", i % 8))
        .name(format!("load-{i:05}"))
        .uid(format!("00000000-0000-4000-8000-{i:012}"))
        .label("app", "oxikube-load")
        .label("pod-template-hash", format!("{:x}", i % 997))
        .image("registry.k8s.io/pause:3.10")
        .created("2026-10-06T12:00:00Z")
        .node(format!("worker-{}", i % 6))
        .ip(format!("10.244.{}.{}", (i / 250) % 250, i % 250 + 2))
        .build()
}

const PODS: usize = 2_000;

/// Retained (bytes, blocks) per pod of whatever `make` returns for each index.
fn per_pod<T>(make: impl Fn(usize) -> T) -> (f64, f64) {
    let before = live();
    let kept: Vec<T> = (0..PODS).map(make).collect();
    let after = live();
    std::hint::black_box(&kept);
    let n = PODS as f64;
    (
        (after.0 - before.0) as f64 / n,
        (after.1 - before.1) as f64 / n,
    )
}

#[test]
fn a_compact_pod_costs_a_fraction_of_a_value_tree() {
    // The inputs, built once and kept out of the measurement.
    let sources: Vec<Resource> = (0..PODS).map(build).collect();

    let (tree_bytes, tree_blocks) = per_pod(|i| sources[i].to_value());
    let (compact_bytes, compact_blocks) = per_pod(|i| JsonDoc::from_value(&sources[i].to_value()));
    let (resource_bytes, resource_blocks) =
        per_pod(|i| Resource::from_json(sources[i].to_value()).expect("a valid pod"));

    eprintln!(
        "per pod: Value tree {tree_bytes:.0} B in {tree_blocks:.0} blocks; \
         JsonDoc {compact_bytes:.0} B in {compact_blocks:.0} blocks; \
         whole Resource (meta + gvk + doc) {resource_bytes:.0} B in {resource_blocks:.0} blocks"
    );

    // The tree is the cost this story removes: the document is at least ten times smaller and
    // one block.
    assert!(
        compact_bytes * 10.0 < tree_bytes,
        "compact {compact_bytes:.0} B vs tree {tree_bytes:.0} B"
    );
    assert!(
        compact_blocks <= 1.5,
        "{compact_blocks} blocks per document"
    );
    // The whole object (typed metadata included) is under a fifth of what the tree alone cost:
    // the shared strings, the shared label set and the Gvk add only a few blocks.
    assert!(
        resource_bytes * 5.0 < tree_bytes,
        "resource {resource_bytes:.0} B vs tree {tree_bytes:.0} B"
    );
    assert!(
        resource_blocks <= 6.0,
        "{resource_blocks} blocks per resource"
    );
}
