//! `notify_coalesced` feeds the `--perf` notify counter (`perf::record_notify`) once per delivered
//! notification. Its own test binary: the recorder is a process-wide `OnceLock`, and other tests
//! notifying in parallel would make the count racy.

use gpui::{AppContext as _, TestAppContext};
use oxikube_runtime::perf::{self, Recorder};
use oxikube_runtime::{FRAME_INTERVAL, notify_coalesced};
use std::sync::Arc;

struct Feed;

#[gpui::test]
fn delivered_coalesced_notifies_are_counted(cx: &mut TestAppContext) {
    let recorder = Arc::new(Recorder::new());
    assert!(
        perf::install(recorder.clone()),
        "first recorder in this process"
    );
    let feed = cx.new(|_| Feed);

    for frame in 1..=3 {
        feed.update(cx, |_, cx| {
            for _ in 0..1_000 {
                notify_coalesced(cx);
            }
        });
        cx.executor().advance_clock(FRAME_INTERVAL);
        cx.run_until_parked();
        assert_eq!(
            recorder.notifies(),
            frame,
            "3 000 calls over 3 frames -> 3 notifies"
        );
    }
}
