//! `notify_coalesced`: many calls per frame produce one observer notification, a 10 000 events/s
//! stream is capped at frame cadence, and released entities are skipped.

use gpui::{AppContext as _, Context, Entity, TestAppContext};
use oxikube_runtime::{FRAME_INTERVAL, NotifyCoalescedExt as _, notify_coalesced, notify_pending};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

/// A model that changes on every event of a stream.
#[derive(Default)]
struct Feed {
    events: u64,
}

impl Feed {
    fn apply_coalesced(&mut self, cx: &mut Context<Self>) {
        self.events += 1;
        notify_coalesced(cx);
    }

    fn apply_eager(&mut self, cx: &mut Context<Self>) {
        self.events += 1;
        cx.notify();
    }
}

/// A feed plus a counter of observer callbacks (what a dependent view would re-render).
fn observed_feed(cx: &mut TestAppContext) -> (Entity<Feed>, Rc<Cell<u32>>) {
    let feed = cx.new(|_| Feed::default());
    let notifications = Rc::new(Cell::new(0));
    let counter = notifications.clone();
    cx.update(|cx| {
        cx.observe(&feed, move |_, _| counter.set(counter.get() + 1))
            .detach();
    });
    (feed, notifications)
}

#[gpui::test]
fn thousand_calls_in_one_frame_notify_observers_once(cx: &mut TestAppContext) {
    let (feed, notifications) = observed_feed(cx);

    feed.update(cx, |feed, cx| {
        for _ in 0..1_000 {
            feed.apply_coalesced(cx);
        }
    });
    cx.run_until_parked();
    assert_eq!(
        notifications.get(),
        0,
        "nothing before the frame interval elapses"
    );
    assert!(cx.update(|cx| notify_pending(cx, feed.entity_id())));

    cx.executor().advance_clock(FRAME_INTERVAL);
    cx.run_until_parked();
    assert_eq!(notifications.get(), 1, "1 000 calls -> one notification");
    assert!(!cx.update(|cx| notify_pending(cx, feed.entity_id())));
    assert_eq!(feed.read_with(cx, |feed, _| feed.events), 1_000);

    // The next frame's calls schedule the next notification.
    feed.update(cx, |_, cx| cx.notify_coalesced());
    cx.executor().advance_clock(FRAME_INTERVAL);
    cx.run_until_parked();
    assert_eq!(notifications.get(), 2);
}

#[gpui::test]
fn ten_thousand_events_per_second_are_capped_at_frame_cadence(cx: &mut TestAppContext) {
    const EVENTS: u32 = 10_000;
    let step = Duration::from_secs(1) / EVENTS;
    let (coalesced, coalesced_count) = observed_feed(cx);
    let (eager, eager_count) = observed_feed(cx);

    for _ in 0..EVENTS {
        coalesced.update(cx, |feed, cx| feed.apply_coalesced(cx));
        eager.update(cx, |feed, cx| feed.apply_eager(cx));
        cx.executor().advance_clock(step);
        cx.run_until_parked();
    }
    cx.executor().advance_clock(FRAME_INTERVAL);
    cx.run_until_parked();

    let frames_per_second =
        (Duration::from_secs(1).as_secs_f64() / FRAME_INTERVAL.as_secs_f64()).ceil() as u32;
    assert_eq!(eager_count.get(), EVENTS, "baseline: one notify per event");
    let coalesced = coalesced_count.get();
    assert!(
        (frames_per_second - 5..=frames_per_second + 1).contains(&coalesced),
        "coalesced to frame cadence: {coalesced} notifications for {EVENTS} events (cap {frames_per_second}/s)"
    );
}

#[gpui::test]
fn released_entity_is_not_notified_and_flag_is_cleared(cx: &mut TestAppContext) {
    let (feed, notifications) = observed_feed(cx);
    let entity_id = feed.entity_id();
    feed.update(cx, |feed, cx| feed.apply_coalesced(cx));
    drop(feed);
    // GPUI releases dropped entities when it next flushes effects.
    cx.update(|_| {});
    cx.run_until_parked();

    cx.executor().advance_clock(FRAME_INTERVAL);
    cx.run_until_parked();
    assert_eq!(notifications.get(), 0);
    assert!(!cx.update(|cx| notify_pending(cx, entity_id)));
}

#[gpui::test]
fn observer_that_streams_again_schedules_the_next_frame(cx: &mut TestAppContext) {
    // An observer reacting to the notify by pushing more work must not be swallowed by a stale flag.
    let (feed, notifications) = observed_feed(cx);
    let relay = feed.clone();
    cx.update(|cx| {
        cx.observe(&feed, move |_, cx| {
            if relay.read(cx).events < 3 {
                relay.update(cx, |feed, cx| feed.apply_coalesced(cx));
            }
        })
        .detach();
    });

    feed.update(cx, |feed, cx| feed.apply_coalesced(cx));
    for _ in 0..3 {
        cx.executor().advance_clock(FRAME_INTERVAL);
        cx.run_until_parked();
    }
    assert_eq!(feed.read_with(cx, |feed, _| feed.events), 3);
    assert_eq!(notifications.get(), 3, "one notification per frame of work");
}
