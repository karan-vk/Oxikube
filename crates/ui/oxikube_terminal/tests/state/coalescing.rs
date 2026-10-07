//! Output is applied on the pump and repaints are frame-coalesced: never one notify per chunk.

use gpui::TestAppContext;
use oxikube_runtime::FRAME_INTERVAL;
use oxikube_testkit::fakes::FakeTerminalBackend;

use super::{harness, next_frame};

#[gpui::test]
fn output_reaches_the_grid_and_repaints_once(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (40, 5));
    backend.output("hello \x1b[1mworld\x1b[0m");
    next_frame(cx);
    assert_eq!(h.row(cx, 0), "hello world");
    assert_eq!(h.notifies.get(), 1);
}

#[gpui::test]
fn a_thousand_chunks_in_one_frame_notify_once(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (80, 24));
    for i in 0..1_000 {
        backend.output(format!("{}", i % 10));
    }
    cx.run_until_parked();
    assert_eq!(h.notifies.get(), 0, "nothing before the frame interval");
    cx.executor().advance_clock(FRAME_INTERVAL);
    cx.run_until_parked();
    assert_eq!(h.notifies.get(), 1, "1 000 chunks -> one notify");

    // Every byte was applied: 1 000 digits fill 12 full rows and 40 cells of the 13th.
    let snapshot = h.terminal.read_with(cx, |terminal, _| terminal.snapshot());
    assert_eq!(snapshot.row_text(0), "0123456789".repeat(8));
    assert_eq!(snapshot.row_text(12), "0123456789".repeat(4));
}

#[gpui::test]
fn a_steady_stream_is_capped_at_frame_cadence(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (80, 24));
    // 2 000 chunks over 200 ms (one every 100 us) is 10 000 chunks/s.
    let step = FRAME_INTERVAL / 83;
    for _ in 0..2_000 {
        backend.output("y\r\n");
        cx.executor().advance_clock(step);
        cx.run_until_parked();
    }
    next_frame(cx);
    let frames = (step * 2_000).as_secs_f64() / FRAME_INTERVAL.as_secs_f64();
    let notifies = f64::from(h.notifies.get());
    assert!(
        notifies <= frames.ceil() + 2.0,
        "{notifies} notifies for {frames:.1} frames of output"
    );
    assert!(
        notifies >= frames.floor() - 2.0,
        "the stream still repaints every frame"
    );
}

#[gpui::test]
fn a_large_chunk_is_applied_in_full(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (80, 24));
    // 1 MiB in one chunk: parsed in slices, all of it lands.
    let mut big = "x".repeat(1 << 20);
    big.push_str("\r\nend");
    backend.output(big);
    next_frame(cx);
    let snapshot = h.terminal.read_with(cx, |terminal, _| terminal.snapshot());
    assert_eq!(snapshot.row_text(snapshot.rows - 1), "end");
    assert_eq!(h.notifies.get(), 1);
}
