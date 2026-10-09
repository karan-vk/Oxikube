//! E05-P599: under a stream and irregular frames (on time, late, stalled) every frame the `--perf`
//! hook records has absorbed at most one coalesced notify per view (`max_view_notifies_per_frame`
//! ≤ 1), and no event waits more than one frame once frames flow. Its own test binary: the
//! recorder is a process-wide `OnceLock`.

use gpui::{AppContext as _, Context, Entity, IntoElement, Render, TestAppContext, Window, div};
use oxikube_runtime::perf::{self, FrameSample, PerfRoot, Recorder};
use oxikube_runtime::{FRAME_INTERVAL, FRAME_STALL, notify_coalesced, notify_pending};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Two streams, as in `tabs-panes` (several clusters' tables churning in one window).
struct Feed;

/// The window's content: redraws when either feed notifies.
struct Panes;

impl Render for Panes {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// A small deterministic generator (xorshift), so the schedule is irregular but reproducible.
struct Schedule(u64);

impl Schedule {
    fn next(&mut self, below: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % below
    }
}

#[gpui::test]
fn irregular_frames_absorb_at_most_one_notify_per_view(cx: &mut TestAppContext) {
    // The first iteration installs it; later ones (`ITERATIONS=n`) reuse it.
    perf::install(Arc::new(Recorder::new()));
    let recorder = perf::global().expect("installed").clone();
    let frames: Rc<RefCell<Vec<FrameSample>>> = Rc::default();
    let feeds: Vec<Entity<Feed>> = (0..2).map(|_| cx.new(|_| Feed)).collect();

    let window = cx.update(|cx| {
        let tap_frames = frames.clone();
        let feeds = feeds.clone();
        let recorder = recorder.clone();
        cx.open_window(Default::default(), move |_, cx| {
            let inner = cx.new(|cx| {
                for feed in &feeds {
                    cx.observe(feed, |_, _, cx| cx.notify()).detach();
                }
                Panes
            });
            cx.new(|_| {
                let mut root = PerfRoot::new(inner, recorder.clone());
                root.set_tap(Some(Rc::new(move |frame| {
                    tap_frames.borrow_mut().push(frame)
                })));
                root
            })
        })
        .unwrap()
    });
    let handle = window.into();
    cx.run_until_parked();
    frames.borrow_mut().clear();

    let mut schedule = Schedule(0x5eed_f00d);
    let step = FRAME_INTERVAL / 8;
    let mut delivered_by_frames = 0u32;
    for refresh in 0..600u32 {
        // Events at 8 points per refresh, on both streams, at irregular rates.
        for _ in 0..8 {
            for feed in &feeds {
                for _ in 0..schedule.next(40) {
                    feed.update(cx, |_, cx| notify_coalesced(cx));
                }
            }
            cx.executor().advance_clock(step);
            cx.run_until_parked();
        }
        // Most refreshes present; some are late by 1 to 5 refreshes; a few stall past the backstop
        // (the window stopped presenting for a while).
        let skip = match schedule.next(100) {
            0..=79 => 0,
            80..=97 => schedule.next(5) + 1,
            _ => 0,
        };
        if refresh % 97 == 96 {
            cx.executor().advance_clock(FRAME_STALL + FRAME_INTERVAL);
            cx.run_until_parked();
        }
        for _ in 0..skip {
            cx.executor().advance_clock(FRAME_INTERVAL);
            cx.run_until_parked();
        }
        let pending = feeds
            .iter()
            .any(|feed| cx.update(|cx| notify_pending(cx, feed.entity_id())));
        cx.update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
        let still_pending = feeds
            .iter()
            .any(|feed| cx.update(|cx| notify_pending(cx, feed.entity_id())));
        if pending && !still_pending {
            delivered_by_frames += 1;
        }
    }

    let frames = frames.borrow();
    assert!(
        frames.len() > 500,
        "the window drew: {} frames",
        frames.len()
    );
    let worst = frames
        .iter()
        .map(|frame| frame.notifies.max_per_view)
        .max()
        .unwrap();
    assert_eq!(
        worst, 1,
        "at most one coalesced notify per view per drawn frame"
    );
    assert!(
        frames
            .iter()
            .all(|frame| frame.notifies.total <= feeds.len() as u64),
        "at most one per feed per frame"
    );
    assert!(
        delivered_by_frames > 500,
        "frames, not the backstop, deliver while they flow: {delivered_by_frames}"
    );
    let tick = recorder.reader().drain(&recorder);
    assert_eq!(tick.max_view_notifies_per_frame, 1);
}
