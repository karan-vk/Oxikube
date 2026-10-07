//! Micro-benchmark (E08-S05): how fast structured log lines are read, on one core.
//!
//! Two costs are measured on a corpus of zap, logrus, bunyan and pino lines plus plain text (one
//! line in ten): `classify`, what a session pays per line as it commits it (the level only), and
//! `parse_line`, what the view pays per row it shows (level, time, message, fields, and the
//! collapsed summary). The budget is 5 000 lines/s without growing latency: the figures here are
//! lines per second per core, so the headroom is their ratio to 5 000.
//!
//! `cargo bench -p oxikube_app --bench log_parse`. Under `cargo test --all-targets` (no `--bench`
//! flag) it runs a small smoke iteration. Not gating.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::time::Instant;

use oxikube_app::logs::parse::{classify, parse_line};

fn corpus() -> Vec<String> {
    let mut lines = Vec::new();
    for i in 0..1_000 {
        lines.push(format!(
            r#"{{"level":"info","ts":1696670400.{i:06},"caller":"server/main.go:42","msg":"handled request {i}","path":"/api/users/{i}","status":200,"duration":0.0123}}"#
        ));
        lines.push(format!(
            r#"{{"level":"warning","msg":"retrying","time":"2023-10-07T09:20:00.{i:03}Z","attempt":{i},"err":"timeout"}}"#
        ));
        lines.push(format!(
            r#"{{"name":"app","hostname":"web-0","pid":18,"level":30,"msg":"tick {i}","time":"2023-10-07T09:20:00.{i:03}Z","v":0}}"#
        ));
        lines.push(format!(
            r#"{{"level":30,"time":16966704{i:05},"pid":1,"hostname":"web-0","req":{{"id":{i},"method":"GET","url":"/health"}},"res":{{"statusCode":200}},"msg":"request completed"}}"#
        ));
        lines.push(format!("2026-10-07 12:00:00 INFO plain text line {i}"));
    }
    lines
}

fn per_second(lines: usize, rounds: usize, run: impl Fn()) -> f64 {
    let start = Instant::now();
    for _ in 0..rounds {
        run();
    }
    (lines * rounds) as f64 / start.elapsed().as_secs_f64()
}

fn main() {
    let rounds = if std::env::args().any(|arg| arg == "--bench") {
        50
    } else {
        1
    };
    let corpus = corpus();
    let bytes: usize = corpus.iter().map(String::len).sum();
    let classify_rate = per_second(corpus.len(), rounds, || {
        for line in &corpus {
            black_box(classify(black_box(line), false));
        }
    });
    let parse_rate = per_second(corpus.len(), rounds, || {
        for line in &corpus {
            if let Some(record) = black_box(parse_line(black_box(line))) {
                black_box(record.summary(200));
            }
        }
    });
    println!(
        "corpus: {} lines, {} bytes ({} bytes/line)",
        corpus.len(),
        bytes,
        bytes / corpus.len()
    );
    println!("classify (per committed line): {classify_rate:>12.0} lines/s on one core");
    println!("parse + summary (per shown row): {parse_rate:>10.0} lines/s on one core");
    println!(
        "headroom over 5 000 lines/s: classify x{:.0}, parse x{:.0}",
        classify_rate / 5_000.0,
        parse_rate / 5_000.0
    );
}
