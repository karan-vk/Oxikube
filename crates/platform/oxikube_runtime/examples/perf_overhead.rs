//! Cost of the `--perf` hot path, on vs off.
//!
//! `cargo run --release -p oxikube_runtime --example perf_overhead`
//!
//! "off" is the global helpers with no recorder installed (what every feed / notify call site pays
//! when `--perf` is not passed); "on" is the same calls with a recorder installed, plus the frame
//! push the `PerfRoot` hook does once per frame. The flush thread is not running here; it never
//! shares a lock with these calls.
#![allow(clippy::print_stdout)]

use oxikube_runtime::perf::{self, Recorder};
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

const N: u32 = 10_000_000;

fn per_call(label: &str, mut f: impl FnMut(u32)) -> f64 {
    // Warm up, then measure.
    for i in 0..N / 10 {
        f(i);
    }
    let start = Instant::now();
    for i in 0..N {
        f(black_box(i));
    }
    let ns = start.elapsed().as_nanos() as f64 / f64::from(N);
    println!("{label:<44} {ns:>7.2} ns/call");
    ns
}

fn main() {
    let off_notify = per_call("record_notify (off: nothing installed)", |_| {
        perf::record_notify()
    });
    let off_feed = per_call("record_feed_deltas (off)", |i| {
        perf::record_feed_deltas(u64::from(i & 7))
    });

    let recorder = Arc::new(Recorder::new());
    perf::install(recorder.clone());
    let on_notify = per_call("record_notify (on)", |_| perf::record_notify());
    let on_feed = per_call("record_feed_deltas (on)", |i| {
        perf::record_feed_deltas(u64::from(i & 7))
    });
    let frame = per_call("Recorder::record_frame (ring push)", |i| {
        std::hint::black_box(recorder.record_frame(Duration::from_nanos(u64::from(i))));
    });
    let instant = per_call("Instant::now + elapsed (hook timing pair)", |_| {
        black_box(Instant::now().elapsed());
    });

    println!();
    println!(
        "notify: +{:.2} ns, feed: +{:.2} ns per call with --perf on; per frame the hook adds \
         ~{:.0} ns (ring push + timing pair) plus one boxed App::defer callback",
        on_notify - off_notify,
        on_feed - off_feed,
        frame + instant
    );
    println!(
        "against an 8 ms frame budget that is {:.5} %",
        (frame + instant) / 8_000_000.0 * 100.0
    );
}
