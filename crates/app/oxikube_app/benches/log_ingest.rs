//! Micro-benchmark (E08-S01): lines through a `LogService` session into its bounded ring, with the
//! delta a viewer reads at the end, and the memory the ring holds.
//!
//! The port is synthetic (an iterator of 100-byte lines that is always ready), so the numbers are
//! the service's own cost: build the entry, batch, commit under the lock, trim the ring. The
//! budget is 5 000 lines/s without dropped frames (docs/PERFORMANCE.md): the "10 minutes at
//! 5 000 lines/s" run is 3 000 000 lines, which this pushes through in a second or two; resident
//! memory afterwards must stay at the ring's bound, not grow with the lines read.
//!
//! `cargo bench -p oxikube_app --bench log_ingest` prints lines/s, batches, the mean batch, the
//! cost of one commit (what the UI thread can wait on the lock for), and RSS before and after.
//! Under `cargo test --all-targets` (no `--bench` flag) it runs a small smoke iteration.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::stream;
use futures::task::LocalSpawnExt;
use jiff::Timestamp;
use oxikube_app::logs::{LogConfig, LogRuntime, LogService, LogState, LogTarget};
use oxikube_domain::OxiResult;
use oxikube_domain::log::LogLine;
use oxikube_ports::{LogOptions, LogPort, LogStream};
use oxikube_testkit::FakeClockPort;
use parking_lot::Mutex;

/// `lines` lines of `pod`'s log, each 100 bytes of text, all ready at once.
struct SyntheticPort {
    lines: usize,
}

#[async_trait]
impl LogPort for SyntheticPort {
    async fn stream_logs(&self, _: &str, pod: &str, _: &LogOptions) -> OxiResult<LogStream> {
        let (pod, container): (Arc<str>, Arc<str>) = (Arc::from(pod), Arc::from("app"));
        let filler = "x".repeat(80);
        let ts = Timestamp::from_second(1_760_000_000).expect("a timestamp");
        Ok(Box::pin(stream::iter((0..self.lines).map(move |i| {
            Ok(LogLine::new(
                ts,
                pod.clone(),
                container.clone(),
                format!("2026-10-03 12:00:00 INFO request {i:08} {filler}"),
            ))
        }))))
    }
}

fn rss_mb() -> f64 {
    let pid = std::process::id().to_string();
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse::<f64>().ok())
        .map_or(f64::NAN, |kb| kb / 1024.0)
}

fn run(lines: usize, buffer_lines: usize) {
    let queue: Arc<Mutex<Vec<BoxFuture<'static, ()>>>> = Arc::default();
    let spawner = {
        let queue = queue.clone();
        Arc::new(move |task: BoxFuture<'static, ()>| queue.lock().push(task))
    };
    let service = LogService::new(
        LogRuntime {
            spawner,
            clock: Arc::new(FakeClockPort::default()),
        },
        LogConfig {
            buffer_lines,
            ..LogConfig::default()
        },
    );
    let before = rss_mb();
    let started = Instant::now();
    let session = service.open(
        Arc::new(SyntheticPort { lines }),
        LogTarget::pod("default", "chatty"),
        LogOptions::follow(),
    );
    let mut pool = LocalPool::new();
    for task in std::mem::take(&mut *queue.lock()) {
        pool.spawner().spawn_local(task).expect("spawn");
    }
    pool.run_until_stalled();
    let elapsed = started.elapsed();
    assert!(
        matches!(session.state(), LogState::Ended(_)),
        "{:?}",
        session.state()
    );

    let (len, dropped) = session.read(|b, _| (b.len(), b.dropped()));
    assert_eq!(len, buffer_lines.min(lines));
    assert_eq!(dropped as usize, lines.saturating_sub(buffer_lines));
    // What a viewer pays per frame: the last 60 rows copied out under the lock.
    let view = Instant::now();
    let rows = session.read(|b, _| {
        b.range(b.len().saturating_sub(60)..b.len())
            .cloned()
            .collect::<Vec<_>>()
    });
    let view = view.elapsed();
    assert_eq!(rows.len(), 60.min(len));

    let batches = session.batches().max(1);
    let per_s = lines as f64 / elapsed.as_secs_f64();
    println!(
        "{lines} lines into a {buffer_lines}-line ring: {:.2?} ({per_s:.0} lines/s, {:.0}x the 5 000/s budget)",
        elapsed,
        per_s / 5_000.0
    );
    println!(
        "  {batches} batches, mean {:.0} lines, {:.1?} per batch (build the entries and commit: an upper bound of the time the lock is held)",
        lines as f64 / batches as f64,
        Duration::from_secs_f64(elapsed.as_secs_f64() / batches as f64),
    );
    println!(
        "  viewport read (60 rows): {view:.1?}; RSS {before:.0} MB -> {:.0} MB",
        rss_mb()
    );
}

fn main() {
    if std::env::args().any(|a| a == "--bench") {
        run(3_000_000, 50_000);
        run(3_000_000, 5_000);
        run(200_000, 1_000_000);
    } else {
        run(10_000, 1_000);
    }
}
