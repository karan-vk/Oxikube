//! Micro-benchmark (E08-S03): the match index over a full ring buffer.
//!
//! Budget (docs/PERFORMANCE.md): keystroke to updated highlights <= 1 frame on a full ring (100 000
//! lines), and streaming 5 000 lines/s with a filter on keeps the p95 frame <= 8 ms. A pattern
//! edit compiles the regex and tests every retained line; the viewer runs that in
//! `CHUNK`-line (16 384) jobs on the background executor, so what the UI thread can wait on is
//! one chunk's hold of the session lock. A streaming delta tests only the lines it appended.
//!
//! `cargo bench -p oxikube_app --bench log_search` prints, per pattern: the compile time, the
//! full scan of 100 000 lines (total and per 16 384-line chunk), and the incremental scan of one
//! second of streaming (5 000 lines). Under `cargo test --all-targets` (no `--bench` flag) it
//! runs a small smoke iteration.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::sync::Arc;
use std::time::Instant;

use jiff::Timestamp;
use oxikube_app::logs::{LogBuffer, LogEntry, LogFilter, MatchIndex};
use oxikube_domain::log::LogLine;

/// The chunk the viewer's background jobs test per lock hold.
const CHUNK: usize = 16_384;

/// A request line of about 100 bytes; every 7th a warning, every 40th an error.
fn line(i: usize) -> LogEntry {
    let text = match i {
        i if i % 40 == 39 => format!("ERROR request {i} failed: upstream payments-svc refused"),
        i if i % 7 == 3 => format!("WARN  GET /api/orders/{i} 200 {}ms: slow query", i % 900),
        i => format!(
            "INFO  GET /api/orders/{i} 200 {}ms user={} region=eu-west-{} trace={i:016x}",
            i % 97,
            i % 4_409,
            i % 3
        ),
    };
    LogEntry::new(LogLine::new(Timestamp::UNIX_EPOCH, "web-0", "app", text))
}

fn buffer(lines: usize) -> LogBuffer {
    let mut buffer = LogBuffer::new(lines);
    buffer.extend((0..lines).map(line));
    buffer
}

fn run(lines: usize, streamed: usize) {
    let mut buffer = buffer(lines);
    println!("{lines}-line ring, {streamed} lines streaming in:");
    let cases = [
        ("literal `ERROR`", LogFilter::new("ERROR")),
        ("regex `WARN|ERROR`", LogFilter::new("WARN|ERROR")),
        (
            "regex `trace=[0-9a-f]+0\\b`",
            LogFilter::new("trace=[0-9a-f]+0\\b"),
        ),
        (
            "case-sensitive literal `slow query`",
            LogFilter {
                pattern: "slow query".to_owned(),
                case_sensitive: true,
                inverse: false,
            },
        ),
        (
            "inverse `INFO`",
            LogFilter {
                pattern: "INFO".to_owned(),
                case_sensitive: false,
                inverse: true,
            },
        ),
    ];
    for (name, filter) in cases {
        let started = Instant::now();
        let matcher = Arc::new(filter.compile().expect("a valid pattern"));
        let compile = started.elapsed();

        // The viewer's background rescan: chunks, each a lock hold.
        let mut index = MatchIndex::new(matcher.clone());
        let started = Instant::now();
        let mut chunks = 0u32;
        let mut slowest = std::time::Duration::ZERO;
        while !index.is_caught_up(&buffer) {
            let chunk = Instant::now();
            index.scan(&buffer, CHUNK);
            slowest = slowest.max(chunk.elapsed());
            chunks += 1;
        }
        let full = started.elapsed();
        let matches = index.len();

        // One second of streaming at 5 000 lines/s on a full ring: the lines are appended (the
        // oldest dropped) and the index brought up to date, as a delta does.
        let started = Instant::now();
        buffer.extend((lines..lines + streamed).map(line));
        let appended = started.elapsed();
        let started = Instant::now();
        let change = index.catch_up(&buffer);
        let incremental = started.elapsed();
        println!(
            "  {name}: compile {compile:.1?}; full scan {full:.1?} in {chunks} chunks (slowest {slowest:.1?}), \
             {matches} matches; +{streamed} lines (ring append {appended:.1?}): index update {incremental:.1?} \
             (+{} / -{} matches)",
            change.appended, change.dropped_front
        );
        // Back to a ring of `lines` for the next pattern.
        buffer = self::buffer(lines);
    }
}

fn main() {
    if std::env::args().any(|a| a == "--bench") {
        run(100_000, 5_000);
        run(10_000, 5_000);
    } else {
        run(2_000, 500);
    }
}
