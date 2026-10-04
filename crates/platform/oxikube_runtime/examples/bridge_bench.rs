//! Micro benchmarks of the runtime bridge (E05-S01).
//!
//! `cargo run --release -p oxikube_runtime --example bridge_bench`
//!
//! 1. `notify_coalesced` call cost: the first call of a frame (flag insert + timer + task spawn)
//!    and the fast path every further call in that frame takes (one hash-set lookup), against a
//!    plain `cx.notify()` inside one update (GPUI already folds repeated notifies of one update
//!    into one observer pass; what it cannot fold is a stream spread over many updates, part 2).
//! 2. A synthetic stream of 10 000 events/s over 5 simulated seconds (GPUI test clock): observer
//!    notifications per second with `cx.notify()` per event vs `notify_coalesced`.
//! 3. `spawn_kube` overhead on a real tokio runtime: spawn + await round trip of a trivial future,
//!    against `tokio::spawn` + `JoinHandle` on the same runtime.
//!
//! Runs on GPUI's test platform (no window), so the numbers are CPU cost only.
#![allow(clippy::print_stdout)]

use gpui::{AppContext as _, Context, TestAppContext};
use oxikube_runtime::{FRAME_INTERVAL, init, notify_coalesced, spawn_kube};
use std::cell::Cell;
use std::hint::black_box;
use std::rc::Rc;
use std::time::{Duration, Instant};

struct Feed;

fn ns_per(elapsed: Duration, n: u32) -> f64 {
    elapsed.as_nanos() as f64 / f64::from(n)
}

fn call_cost(cx: &mut TestAppContext) {
    const N: u32 = 1_000_000;
    let feed = cx.new(|_| Feed);
    // One observer, so each delivered notify pays a realistic observer pass.
    let counter = Rc::new(Cell::new(0u64));
    cx.update(|cx| {
        cx.observe(&feed, move |_, _| counter.set(counter.get() + 1))
            .detach()
    });

    // First call of a frame, repeated across N / 100 frames.
    let mut first = Duration::ZERO;
    for _ in 0..N / 100 {
        feed.update(cx, |_, cx: &mut Context<Feed>| {
            let start = Instant::now();
            notify_coalesced(cx);
            first += start.elapsed();
        });
        cx.executor().advance_clock(FRAME_INTERVAL);
        cx.run_until_parked();
    }

    // Fast path: N calls inside one frame.
    let fast = feed.update(cx, |_, cx: &mut Context<Feed>| {
        notify_coalesced(cx);
        let start = Instant::now();
        for _ in 0..N {
            notify_coalesced(black_box(&mut *cx));
        }
        start.elapsed()
    });
    cx.executor().advance_clock(FRAME_INTERVAL);
    cx.run_until_parked();

    // Plain cx.notify() inside one update, plus the single observer pass when it flushes.
    let start = Instant::now();
    feed.update(cx, |_, cx: &mut Context<Feed>| {
        for _ in 0..N {
            cx.notify();
        }
    });
    let eager = start.elapsed();

    println!(
        "notify_coalesced, first call of a frame      {:>8.1} ns/call",
        ns_per(first, N / 100)
    );
    println!(
        "notify_coalesced, further calls in the frame {:>8.1} ns/call",
        ns_per(fast, N)
    );
    println!(
        "cx.notify(), N calls in one update           {:>8.1} ns/call",
        ns_per(eager, N)
    );
}

fn stream(cx: &mut TestAppContext) {
    const RATE: u32 = 10_000;
    const SECONDS: u32 = 5;
    let step = Duration::from_secs(1) / RATE;
    let coalesced = cx.new(|_| Feed);
    let eager = cx.new(|_| Feed);
    let counts = [Rc::new(Cell::new(0u64)), Rc::new(Cell::new(0u64))];
    for (entity, count) in [(&coalesced, &counts[0]), (&eager, &counts[1])] {
        let count = count.clone();
        cx.update(|cx| {
            cx.observe(entity, move |_, _| count.set(count.get() + 1))
                .detach()
        });
    }

    for _ in 0..RATE * SECONDS {
        coalesced.update(cx, |_, cx| notify_coalesced(cx));
        eager.update(cx, |_, cx| cx.notify());
        cx.executor().advance_clock(step);
        cx.run_until_parked();
    }
    let secs = f64::from(SECONDS);
    println!(
        "10 000 events/s for {SECONDS} s: cx.notify() {:.0} notifies/s, notify_coalesced {:.0} notifies/s (frame interval {:.3} ms)",
        counts[1].get() as f64 / secs,
        counts[0].get() as f64 / secs,
        FRAME_INTERVAL.as_secs_f64() * 1e3
    );
}

fn spawn_overhead(cx: &mut TestAppContext) {
    const N: u32 = 20_000;
    cx.executor().allow_parking();
    cx.update(|cx| init(cx)).expect("tokio runtime");
    let handle = cx
        .update(|cx| oxikube_runtime::handle(cx))
        .expect("tokio mode");

    let executor = cx.foreground_executor().clone();
    let start = Instant::now();
    for i in 0..N {
        let task = cx.update(|cx| spawn_kube(cx, async move { black_box(i) }));
        executor.block_test(task).expect("completes");
    }
    let bridged = start.elapsed();

    let start = Instant::now();
    for i in 0..N {
        handle
            .block_on(handle.spawn(async move { black_box(i) }))
            .expect("completes");
    }
    let raw = start.elapsed();

    println!(
        "spawn_kube spawn + await round trip           {:>8.1} us/task",
        ns_per(bridged, N) / 1e3
    );
    println!(
        "tokio::spawn + block_on round trip            {:>8.1} us/task",
        ns_per(raw, N) / 1e3
    );
}

fn main() {
    call_cost(&mut TestAppContext::single());
    stream(&mut TestAppContext::single());
    spawn_overhead(&mut TestAppContext::single());
}
