//! Micro-benchmark (E11-S04): the cost of `AliasTable::resolve`, which the jump bar pays on every
//! keystroke, and of rebuilding the table after discovery (a background task).
//!
//! `cargo bench -p oxikube_app --bench alias_resolve` prints min / median / p95 per call against a
//! stock cluster plus 500 CRDs. Under `cargo test --all-targets` (no `--bench` flag) it runs one
//! small iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::time::{Duration, Instant};

use oxikube_app::search::aliases::AliasTable;
use oxikube_testkit::kinds::{core_kinds, kind};

const BATCH: usize = 1_000;

fn report(label: &str, mut per_call: Vec<Duration>) {
    per_call.sort();
    let at = |q: f64| per_call[((per_call.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<40} min {:>8.0?}  median {:>8.0?}  p95 {:>8.0?}",
        at(0.0),
        at(0.5),
        at(0.95)
    );
}

fn per_call(samples: usize, batch: usize, mut call: impl FnMut()) -> Vec<Duration> {
    (0..samples)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..batch {
                call();
            }
            start.elapsed() / u32::try_from(batch).unwrap_or(u32::MAX)
        })
        .collect()
}

fn main() {
    let smoke = !std::env::args().any(|a| a == "--bench");
    let samples = if smoke { 2 } else { 200 };
    let batch = if smoke { 10 } else { BATCH };

    let mut kinds = core_kinds();
    for i in 0..500 {
        kinds.push(
            kind(
                &format!("g{}.example.io", i % 40),
                "v1",
                &format!("Kind{i}"),
                &format!("kind{i}s"),
            )
            .short(&format!("k{i}"))
            .build(),
        );
    }
    let table = AliasTable::new();
    table.set_discovered(&kinds);
    println!("{} names from {} kinds", table.len(), kinds.len());

    report(
        "resolve: built-in (po)",
        per_call(samples, batch, || {
            black_box(table.resolve(black_box("po")));
        }),
    );
    report(
        "resolve: mixed case discovery (Kind250S)",
        per_call(samples, batch, || {
            black_box(table.resolve(black_box("Kind250S")));
        }),
    );
    report(
        "resolve: unknown with suggestions",
        per_call(samples, batch.min(100), || {
            black_box(table.resolve(black_box("deploymnt")));
        }),
    );
    report(
        "rebuild: set_discovered (all kinds)",
        per_call(samples.min(30), 1, || {
            table.set_discovered(black_box(&kinds))
        }),
    );
    let one = kind("example.io", "v1", "Extra", "extras").build();
    report(
        "rebuild: one CRD changed",
        per_call(samples.min(30), 1, || {
            table.apply_kinds_change(&[], std::slice::from_ref(black_box(&one)));
        }),
    );
}
