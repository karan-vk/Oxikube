//! E11-S04: a jump-bar keystroke looks a name up in the alias table, so a lookup of a known
//! name must not allocate (ADR 0013: input within one frame, no garbage per keystroke).
//!
//! This binary counts the allocations of the calling thread through a counting global allocator,
//! so it holds exactly one test and nothing else runs in the process.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use oxikube_app::search::aliases::{AliasTable, Resolution};
use oxikube_domain::AliasTarget;
use oxikube_testkit::kinds::{cert_manager_kinds, clashing_crds, core_kinds};

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct Counting;

// SAFETY: forwards every call to the system allocator unchanged; the counter is a thread-local
// `Cell` with a const initialiser, so touching it never allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which we pass on.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` come from a matching `alloc` of `System`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations_of(work: impl FnOnce()) -> usize {
    let before = ALLOCATIONS.with(Cell::get);
    work();
    ALLOCATIONS.with(Cell::get) - before
}

#[test]
fn looking_up_a_known_name_allocates_nothing() {
    // The counter works: an unknown name builds a suggestion list and a string.
    assert!(allocations_of(|| drop(vec![0u8; 16])) > 0);
    let table = AliasTable::new();
    let mut kinds = core_kinds();
    kinds.extend(cert_manager_kinds());
    kinds.extend(clashing_crds());
    table.set_discovered(&kinds);
    table.set_user_aliases([(
        "fred".to_owned(),
        AliasTarget::command("pod", ["fred".to_owned(), "app=blee".to_owned()]),
    )]);

    // Warm up anything lazily initialised on first use.
    let _ = table.resolve("po");

    for word in [
        "po",
        "DP",
        " Deployments ",
        "certificates.example.io",
        "wg",
        "fred",
        "certificates", // ambiguous: shares the candidate list, no copy
    ] {
        let mut resolution = None;
        let allocated = allocations_of(|| resolution = Some(table.resolve(word)));
        assert!(
            resolution.as_ref().is_some_and(Resolution::is_known),
            "{word:?} should be known"
        );
        assert_eq!(allocated, 0, "resolving {word:?} allocated");
    }
}
