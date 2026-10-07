//! Micro-benchmark (E08-S04): the lines of many pods through an aggregate session, merged by
//! server timestamp into one bounded ring.
//!
//! The log port is synthetic (every pod's stream is an iterator of 100-byte lines that is always
//! ready, stamped so the pods interleave), the pods come from a `FakeResourcePort` watch, and the
//! clock is virtual: it is advanced past the reorder window until the aggregate has drained. So
//! the numbers are the aggregate's own cost: the per-stream batching, the hop to the coordinator,
//! the heap merge and the commit under the lock. The budget is 5 000 lines/s in total without
//! dropped frames (docs/PERFORMANCE.md): the merged buffer must keep far ahead of that, and
//! resident memory must stay at the ring's bound, not grow with the lines read.
//!
//! `cargo bench -p oxikube_app --bench log_merge` prints lines/s, the share of lines that came
//! out of timestamp order (the window's best effort), the peak heap the run held (a counting
//! allocator) and RSS before and after. Under
//! `cargo test --all-targets` (no `--bench` flag) it runs a small smoke iteration.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::stream;
use futures::task::LocalSpawnExt;
use jiff::Timestamp;
use oxikube_app::logs::{
    AggregatePorts, AggregateSpec, LogConfig, LogRuntime, LogService, LogState,
};
use oxikube_domain::OxiResult;
use oxikube_domain::log::LogLine;
use oxikube_ports::{LogOptions, LogPort, LogStream};
use oxikube_testkit::{FakeClockPort, FakeResourcePort, pod};
use parking_lot::Mutex;

/// Counts the bytes the process holds on the heap, and the most it held since the last
/// [`reset_peak`]: the memory the aggregate needs, which the resident set size (a high-water mark
/// the allocator keeps) cannot say.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards to the system allocator and only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(live, Ordering::Relaxed);
        // SAFETY: the caller's layout, unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: the pointer and layout came from `alloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn reset_peak() {
    PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
}

fn peak_mb() -> f64 {
    PEAK.load(Ordering::Relaxed) as f64 / (1024.0 * 1024.0)
}

/// `lines` lines for every pod, 100 bytes of text, all ready at once. Pod `web-N` is the Nth
/// stream; its line `i` is stamped `i * pods + N` milliseconds, so the pods interleave.
struct SyntheticPort {
    lines: usize,
    pods: usize,
}

#[async_trait]
impl LogPort for SyntheticPort {
    async fn stream_logs(&self, _: &str, pod: &str, _: &LogOptions) -> OxiResult<LogStream> {
        let (pod, container): (Arc<str>, Arc<str>) = (Arc::from(pod), Arc::from("app"));
        let n: usize = pod
            .rsplit('-')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        let pods = self.pods;
        let filler = "x".repeat(80);
        let base = Timestamp::from_second(1_760_000_000).expect("a timestamp");
        Ok(Box::pin(stream::iter((0..self.lines).map(move |i| {
            let ts = base
                .checked_add(Duration::from_millis((i * pods + n) as u64))
                .expect("a timestamp");
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

fn run(pods: usize, lines: usize, buffer_lines: usize) {
    let queue: Arc<Mutex<Vec<BoxFuture<'static, ()>>>> = Arc::default();
    let spawner = {
        let queue = queue.clone();
        Arc::new(move |task: BoxFuture<'static, ()>| queue.lock().push(task))
    };
    let clock = Arc::new(FakeClockPort::default());
    let service = LogService::new(
        LogRuntime {
            spawner,
            clock: clock.clone(),
        },
        LogConfig {
            buffer_lines,
            max_streams: pods,
            ..LogConfig::default()
        },
    );
    let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
    resources.insert(
        oxikube_testkit::deployment()
            .name("web")
            .namespace("default")
            .build(),
    );
    for n in 0..pods {
        resources.insert(
            pod()
                .name(format!("web-{n}"))
                .namespace("default")
                .uid(format!("uid-{n}"))
                .label("app", "web")
                .build(),
        );
    }
    let ports = AggregatePorts {
        logs: Arc::new(SyntheticPort { lines, pods }),
        resources,
    };
    let spec = AggregateSpec::selector("default", "app=web");

    let before = rss_mb();
    reset_peak();
    let started = Instant::now();
    let session = service.open_aggregate(
        ports,
        spec,
        LogOptions {
            follow: false,
            ..LogOptions::default()
        },
    );
    let mut pool = LocalPool::new();
    let mut rounds = 0;
    while !matches!(session.state(), LogState::Ended(_)) {
        for task in std::mem::take(&mut *queue.lock()) {
            pool.spawner().spawn_local(task).expect("spawn");
        }
        pool.run_until_stalled();
        clock.advance(Duration::from_millis(100));
        rounds += 1;
        assert!(rounds < 100_000, "the aggregate never drained");
    }
    let elapsed = started.elapsed();

    let total = pods * lines;
    let (len, dropped, inversions) = session.read(|b, _| {
        let mut inversions = 0usize;
        let mut last = None;
        for entry in b.iter() {
            if last.is_some_and(|ts| entry.ts < ts) {
                inversions += 1;
            }
            last = Some(entry.ts);
        }
        (b.len(), b.dropped(), inversions)
    });
    assert_eq!(len, buffer_lines.min(total));
    assert_eq!(dropped as usize, total.saturating_sub(buffer_lines));

    let per_s = total as f64 / elapsed.as_secs_f64();
    println!(
        "{pods} pods x {lines} lines into a {buffer_lines}-line ring: {:.2?} ({per_s:.0} lines/s, {:.0}x the 5 000/s budget)",
        elapsed,
        per_s / 5_000.0
    );
    println!(
        "  {} of the {len} lines kept are out of timestamp order ({:.2}%); peak heap {:.1} MB; RSS {before:.0} MB -> {:.0} MB",
        inversions,
        100.0 * inversions as f64 / len.max(1) as f64,
        peak_mb(),
        rss_mb()
    );
}

fn main() {
    if std::env::args().any(|a| a == "--bench") {
        run(10, 30_000, 50_000);
        run(20, 15_000, 50_000);
        run(5, 10_000, 1_000_000);
        run(10, 30_000, 5_000);
    } else {
        run(3, 500, 1_000);
    }
}
