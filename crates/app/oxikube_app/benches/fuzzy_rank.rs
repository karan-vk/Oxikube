//! Micro-benchmark (E11-S11): ranking 2 000 command titles for each keystroke of a query, as the
//! command palette does with `FuzzyService`.
//!
//! The palette filters up to 2 000 entries inside a 5 ms budget (`docs/PERFORMANCE.md`). Titles
//! are shaped like the palette's rows (`"{category} {title}"`, some with an object name).
//! `cargo bench -p oxikube_app --bench fuzzy_rank` prints min / p50 / p95 / max per keystroke and
//! fails if the p95 of any exceeds the budget; under `cargo test --all-targets` it runs one pass as
//! a smoke test. `--recents` ranks with 50 recent commands, as the palette does.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::time::{Duration, Instant};

use oxikube_app::search::fuzzy::FuzzyService;

const CANDIDATES: usize = 2_000;
const BUDGET: Duration = Duration::from_millis(5);

fn titles(n: usize) -> Vec<String> {
    const CATEGORIES: [&str; 10] = [
        "Pod",
        "Workload",
        "Node",
        "Cluster",
        "Namespace",
        "View",
        "Logs",
        "Helm",
        "Terminal",
        "Resource",
    ];
    const VERBS: [&str; 16] = [
        "Delete",
        "Scale",
        "Restart",
        "Cordon",
        "Drain",
        "Shell",
        "Attach",
        "Describe",
        "View YAML",
        "Edit",
        "Port Forward",
        "Rollout Undo",
        "Toggle Read-only",
        "Copy Name",
        "Open Logs",
        "Autoscale",
    ];
    (0..n)
        .map(|i| {
            let category = CATEGORIES[i % CATEGORIES.len()];
            let verb = VERBS[(i / CATEGORIES.len()) % VERBS.len()];
            let round = i / (CATEGORIES.len() * VERBS.len());
            if round == 0 {
                format!("{category} {verb}")
            } else {
                format!("{category} {verb} extension-{round:03}")
            }
        })
        .collect()
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let bench = std::env::args().any(|a| a == "--bench");
    let with_recents = std::env::args().any(|a| a == "--recents");
    let rounds = if bench { 500 } else { 1 };

    let titles = titles(CANDIDATES);
    let service = FuzzyService::new();
    // The recents: every 40th command, most recent first.
    let recency = |ix: usize| ix.is_multiple_of(40).then_some(ix / 40).filter(|r| *r < 50);
    let items: Vec<(usize, &str)> = titles
        .iter()
        .enumerate()
        .map(|(ix, title)| (ix, title.as_str()))
        .collect();

    println!(
        "FuzzyService::rank over {CANDIDATES} titles{} ({rounds} rounds)",
        if with_recents { ", with recents" } else { "" }
    );
    let mut worst = Duration::ZERO;
    for query in [
        "p", "po", "pod", "pod s", "pod sh", "scale", "wl scale", "drain nd", "zzz",
    ] {
        let mut times = Vec::with_capacity(rounds);
        let mut matched = 0;
        for _ in 0..rounds {
            let start = Instant::now();
            matched = black_box(service.rank_with(
                black_box(query),
                &items,
                usize::MAX,
                |(_, title)| title,
                |(ix, _)| if with_recents { recency(*ix) } else { None },
            ))
            .len();
            times.push(start.elapsed());
        }
        times.sort();
        let p95 = percentile(&times, 0.95);
        worst = worst.max(p95);
        println!(
            "  {query:<10} {matched:>5} matched  min {:>9.1?}  p50 {:>9.1?}  p95 {:>9.1?}  max {:>9.1?}",
            times[0],
            percentile(&times, 0.5),
            p95,
            times[times.len() - 1],
        );
    }
    if bench {
        assert!(
            worst <= BUDGET,
            "p95 {worst:?} is over the {BUDGET:?} budget"
        );
    }
}
