//! Benchmark (E10-S10): the YAML to JSON and JSON to YAML conversions, and the strict JSON check
//! JSON mode adds to the validation pass, over a 2k-line multi-document manifest and a 5 MB
//! `kubectl get -o yaml` / `-o json` style dump. All of it runs on the background executor in the
//! editor; the UI thread only takes the rope snapshot and, at the end, replaces the text.
//!
//! `cargo bench -p oxikube_editor --bench convert` prints min / median / p95 per run. Under
//! `cargo test --all-targets` (no `--bench` flag) it runs one small iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::time::{Duration, Instant};

use oxikube_editor::json::{first_error, json_to_yaml, yaml_to_json};

#[path = "support/mod.rs"]
mod support;

use support::{dump, manifest};

fn time(label: &str, bytes: usize, samples: usize, mut run: impl FnMut()) {
    let mut times: Vec<Duration> = (0..samples)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed()
        })
        .collect();
    times.sort();
    let at = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<34} {bytes:>9} B  min {:>9.2?}  median {:>9.2?}  p95 {:>9.2?}",
        at(0.0),
        at(0.5),
        at(0.95),
    );
}

fn main() {
    let smoke = !std::env::args().any(|a| a == "--bench");
    let (samples, big) = if smoke {
        (1, 64 * 1024)
    } else {
        (20, 5 * 1024 * 1024)
    };

    for (name, yaml) in [
        ("2k-line manifest", manifest(2_000)),
        ("5 MB get -o yaml dump", dump(big)),
    ] {
        let json = yaml_to_json(&yaml).expect("converts").text;
        time(
            &format!("{name}: YAML to JSON"),
            yaml.len(),
            samples,
            || {
                black_box(yaml_to_json(black_box(&yaml)).expect("converts"));
            },
        );
        time(
            &format!("{name}: JSON to YAML"),
            json.len(),
            samples,
            || {
                black_box(json_to_yaml(black_box(&json)).expect("converts"));
            },
        );
        time(
            &format!("{name}: strict JSON check"),
            json.len(),
            samples,
            || {
                assert_eq!(
                    black_box(first_error(black_box(&json))),
                    None,
                    "{name}: the generated JSON must be strictly valid"
                );
            },
        );
    }
}
