//! The latency budget: ranking 2 000 candidates takes at most 5 ms (`docs/PERFORMANCE.md`).
//!
//! The assertion has generous slack so a loaded CI machine does not flake: a debug build (what
//! `cargo test` makes) is allowed 20x, a release build 4x. `cargo bench -p oxikube_app --bench
//! fuzzy_rank` holds the real 5 ms and prints the numbers.

use std::time::{Duration, Instant};

use crate::search::fuzzy::FuzzyService;

const BUDGET: Duration = Duration::from_millis(5);

fn slack() -> u32 {
    if cfg!(debug_assertions) { 20 } else { 4 }
}

fn titles(n: usize) -> Vec<String> {
    const CATEGORIES: [&str; 8] = [
        "Pod",
        "Workload",
        "Node",
        "Cluster",
        "Namespace",
        "View",
        "Logs",
        "Helm",
    ];
    const VERBS: [&str; 8] = [
        "Delete",
        "Scale",
        "Restart",
        "Cordon",
        "Drain",
        "Shell",
        "Describe",
        "View YAML",
    ];
    (0..n)
        .map(|i| {
            format!(
                "{} {} extension-{:03}",
                CATEGORIES[i % 8],
                VERBS[(i / 8) % 8],
                i / 64
            )
        })
        .collect()
}

#[test]
fn two_thousand_candidates_rank_inside_the_budget() {
    let titles = titles(2_000);
    let service = FuzzyService::new();
    // Warm the pool, then take the best of a few runs per query: this is a budget, not a race.
    service.rank("pod", titles.as_slice(), usize::MAX);
    for query in ["p", "pod s", "wl scale", "drain nd", "zzz"] {
        let best = (0..5)
            .map(|_| {
                let start = Instant::now();
                let found = service.rank(query, titles.as_slice(), usize::MAX);
                std::hint::black_box(found);
                start.elapsed()
            })
            .min()
            .unwrap();
        assert!(
            best <= BUDGET * slack(),
            "{query:?}: {best:?} over {:?}",
            BUDGET * slack()
        );
    }
}

#[test]
fn a_blank_query_over_two_thousand_is_cheap() {
    let titles = titles(2_000);
    let service = FuzzyService::new();
    let start = Instant::now();
    let found = service.rank("", titles.as_slice(), usize::MAX);
    assert_eq!(found.len(), 2_000);
    assert!(start.elapsed() <= BUDGET * slack());
}
