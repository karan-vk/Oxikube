//! `notify_coalesced` with a window (E05-P599): deliveries are paced by the window's frames, not by
//! a timer. A frame is simulated the way the platform delivers one: GPUI runs the window's
//! next-frame callbacks (`Window::simulate_next_frame`), then draws the window if it is dirty (the
//! test platform draws when that update flushes).

use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, IntoElement, Render, TestAppContext, Window,
    div,
};
use oxikube_runtime::{FRAME_INTERVAL, FRAME_STALL, notify_coalesced, notify_pending};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

/// A stream's model.
#[derive(Default)]
struct Feed {
    events: u64,
}

/// The view a window shows; counts its renders (one per drawn frame it is dirty in).
struct FeedView {
    renders: Rc<Cell<u32>>,
}

impl Render for FeedView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div()
    }
}

/// A window, a feed, and counters for the feed's notifications and the view's renders.
struct Fixture {
    window: AnyWindowHandle,
    feed: Entity<Feed>,
    notifications: Rc<Cell<u32>>,
    renders: Rc<Cell<u32>>,
}

impl Fixture {
    fn new(cx: &mut TestAppContext) -> Self {
        let feed = cx.new(|_| Feed::default());
        let notifications = Rc::new(Cell::new(0));
        let renders = Rc::new(Cell::new(0));
        let window = cx.update(|cx| {
            let counter = notifications.clone();
            cx.observe(&feed, move |_, _| counter.set(counter.get() + 1))
                .detach();
            let renders = renders.clone();
            let feed = feed.clone();
            cx.open_window(Default::default(), move |_, cx| {
                cx.new(|cx| {
                    // The view redraws when the feed notifies, like a table over its store.
                    cx.observe(&feed, |_, _, cx| cx.notify()).detach();
                    FeedView { renders }
                })
            })
            .unwrap()
            .into()
        });
        cx.run_until_parked();
        Self {
            window,
            feed,
            notifications,
            renders,
        }
    }

    /// `n` stream events, each in its own update (as a channel drain delivers them).
    fn events(&self, n: u64, cx: &mut TestAppContext) {
        for _ in 0..n {
            self.feed.update(cx, |feed, cx| {
                feed.events += 1;
                notify_coalesced(cx);
            });
        }
        cx.run_until_parked();
    }

    /// One frame of the window; returns how many next-frame callbacks it ran.
    fn frame(&self, cx: &mut TestAppContext) -> usize {
        let ran = cx
            .update_window(self.window, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
        ran
    }

    fn elapse(&self, by: Duration, cx: &mut TestAppContext) {
        cx.executor().advance_clock(by);
        cx.run_until_parked();
    }

    /// Frames are flowing: one batch delivered by a frame.
    fn presenting(&self, cx: &mut TestAppContext) {
        self.events(1, cx);
        self.frame(cx);
        assert_eq!(self.notifications.get(), 1, "the first frame delivers");
    }
}

#[gpui::test]
fn many_events_between_two_frames_give_one_notify(cx: &mut TestAppContext) {
    let f = Fixture::new(cx);
    f.presenting(cx);
    let renders = f.renders.get();

    // A thousand events spread over most of a refresh: nothing until the frame.
    for _ in 0..10 {
        f.events(100, cx);
        f.elapse(FRAME_INTERVAL / 12, cx);
    }
    assert_eq!(f.notifications.get(), 1, "no timer delivers between frames");
    assert!(cx.update(|cx| notify_pending(cx, f.feed.entity_id())));

    f.frame(cx);
    assert_eq!(f.notifications.get(), 2, "1 000 events -> one notify");
    assert_eq!(f.renders.get(), renders + 1, "drawn in that frame");
    assert!(!cx.update(|cx| notify_pending(cx, f.feed.entity_id())));
    assert_eq!(f.feed.read_with(cx, |feed, _| feed.events), 1_001);

    // A frame with nothing pending notifies nothing.
    f.frame(cx);
    assert_eq!(f.notifications.get(), 2);
}

#[gpui::test]
fn a_late_frame_still_gives_one_notify(cx: &mut TestAppContext) {
    let f = Fixture::new(cx);
    f.presenting(cx);

    // The next frame comes 6 refreshes late, events streaming all along: a timer-paced notify
    // would have fired 6 times before it.
    for _ in 0..6 {
        f.events(50, cx);
        f.elapse(FRAME_INTERVAL, cx);
    }
    assert_eq!(f.notifications.get(), 1, "the late frame is waited for");

    f.frame(cx);
    assert_eq!(f.notifications.get(), 2, "one notify in the late frame");
    f.events(1, cx);
    f.frame(cx);
    assert_eq!(f.notifications.get(), 3, "and one in the next");
}

#[gpui::test]
fn an_event_right_after_a_frame_is_in_the_next_frame(cx: &mut TestAppContext) {
    let f = Fixture::new(cx);
    f.presenting(cx);

    // No clock advance at all: the frame alone delivers, nothing waits on a timer.
    for frame in 2..=5 {
        f.events(1, cx);
        f.frame(cx);
        assert_eq!(f.notifications.get(), frame);
    }
}

#[gpui::test]
fn without_frames_the_backstop_delivers_and_the_next_frame_does_not_double(
    cx: &mut TestAppContext,
) {
    let f = Fixture::new(cx);
    f.presenting(cx);

    // The window stops presenting (hidden, minimised): the backstop delivers after FRAME_STALL.
    f.events(10, cx);
    f.elapse(FRAME_STALL - Duration::from_millis(1), cx);
    assert_eq!(f.notifications.get(), 1);
    f.elapse(Duration::from_millis(1), cx);
    assert_eq!(f.notifications.get(), 2, "the backstop delivered");

    // Long after the last frame the backstop is the cadence: one per FRAME_INTERVAL.
    f.events(10, cx);
    f.elapse(FRAME_INTERVAL, cx);
    assert_eq!(f.notifications.get(), 3);

    // The window presents again with more events pending. Its frame draws the backstop's notify,
    // so the new batch waits for the frame after it: still one notify per drawn frame.
    f.events(10, cx);
    f.frame(cx);
    assert_eq!(f.notifications.get(), 3, "no second notify in that frame");
    f.frame(cx);
    assert_eq!(f.notifications.get(), 4, "delivered in the next frame");
}

#[gpui::test]
fn a_window_that_is_not_presenting_holds_one_hook(cx: &mut TestAppContext) {
    let f = Fixture::new(cx);
    // Many batches, each delivered by the backstop, while the window draws no frame.
    for _ in 0..50 {
        f.events(3, cx);
        f.elapse(FRAME_INTERVAL, cx);
    }
    assert_eq!(f.notifications.get(), 50);
    assert_eq!(f.frame(cx), 1, "one frame hook queued, not one per batch");
}

#[gpui::test]
fn released_entity_is_skipped_at_the_frame(cx: &mut TestAppContext) {
    let f = Fixture::new(cx);
    let gone = cx.new(|_| Feed::default());
    let gone_id = gone.entity_id();
    let notified = Rc::new(Cell::new(0));
    let counter = notified.clone();
    cx.update(|cx| {
        cx.observe(&gone, move |_, _| counter.set(counter.get() + 1))
            .detach();
    });
    gone.update(cx, |_, cx| notify_coalesced(cx));
    f.events(1, cx);
    drop(gone);
    cx.update(|_| {});
    cx.run_until_parked();

    f.frame(cx);
    assert_eq!(notified.get(), 0);
    assert_eq!(
        f.notifications.get(),
        1,
        "the live entity is still notified"
    );
    assert!(!cx.update(|cx| notify_pending(cx, gone_id)));
}

#[gpui::test]
fn an_event_inside_the_window_update_still_waits_for_the_frame(cx: &mut TestAppContext) {
    // A view's handler streams while its own window is being updated (and so cannot be updated
    // again): the frame hook is queued once that update returns.
    let f = Fixture::new(cx);
    f.presenting(cx);
    let feed = f.feed.clone();
    cx.update_window(f.window, |_, _, cx| {
        feed.update(cx, |feed, cx| {
            feed.events += 1;
            notify_coalesced(cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.notifications.get(), 1);
    assert_eq!(f.frame(cx), 1, "the window's frame hook was queued");
    assert_eq!(f.notifications.get(), 2);
}
