//! The live tail: new events on top in the same feed, pausing counts them and resuming shows them
//! in order, scrolling away from the top pauses and back resumes, and the feed's state.

use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, point, px};
use oxikube_domain::command::Command;
use oxikube_testkit::FakeEventFeed;

use super::{Fx, cluster, ev, seed, warn};
use crate::model::Tail;

fn feed() -> FakeEventFeed {
    FakeEventFeed::new()
        .initial(seed())
        .add(1, warn("w3", "default", "web-1", 10, "Unhealthy"))
        .add(2, ev("n3", "default", "web-1", 11, "Killing"))
}

#[gpui::test]
fn new_events_arrive_on_top(cx: &mut TestAppContext) {
    let mut fx = Fx::with_feed(cx, feed());
    fx.open();
    assert_eq!(fx.uids(), ["w2", "n2", "w1", "n1"]);
    fx.tick();
    assert_eq!(fx.uids(), ["w3", "w2", "n2", "w1", "n1"]);
    assert_eq!(
        fx.port.live_feeds(),
        1,
        "the view reads the cluster's one feed"
    );
}

#[gpui::test]
fn a_paused_tail_counts_new_events_and_resumes_in_order(cx: &mut TestAppContext) {
    let mut fx = Fx::with_feed(cx, feed());
    fx.open();
    fx.keys("p");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::EventsTogglePause { cluster: cluster() })
    );
    assert_eq!(fx.model(|m| m.tail()), Tail::Paused { auto: false });
    fx.tick();
    fx.tick();
    assert_eq!(fx.uids(), ["w2", "n2", "w1", "n1"], "nothing moves");
    assert_eq!(fx.model(|m| m.unseen()), 2);
    assert!(fx.drawn("events-unseen"));
    // A filter change while paused still shows its rows.
    fx.keys("ctrl-z");
    assert_eq!(fx.uids(), ["w3", "w2", "w1"]);
    fx.keys("ctrl-z");
    fx.keys("p");
    assert_eq!(fx.model(|m| m.tail()), Tail::Live);
    assert_eq!(fx.uids(), ["n3", "w3", "w2", "n2", "w1", "n1"]);
    assert!(!fx.drawn("events-unseen"));
}

#[gpui::test]
fn clicking_the_new_events_chip_resumes(cx: &mut TestAppContext) {
    let mut fx = Fx::with_feed(cx, feed());
    fx.open();
    fx.click("events-pause");
    fx.tick();
    assert_eq!(fx.uids().len(), 4);
    fx.click("events-unseen");
    assert_eq!(fx.uids().len(), 5);
    assert_eq!(fx.model(|m| m.tail()), Tail::Live);
}

#[gpui::test]
fn scrolling_away_from_the_top_pauses_and_back_resumes(cx: &mut TestAppContext) {
    let many: Vec<_> = (0..300)
        .map(|i| ev(&format!("e{i:03}"), "default", "web-0", i, "Pulled"))
        .collect();
    let mut fx = Fx::with_feed(
        cx,
        FakeEventFeed::new()
            .initial(many)
            .add(1, warn("late", "default", "web-0", 1_000, "BackOff")),
    );
    fx.open();
    let bounds = fx.vcx.debug_bounds("events-table").expect("drawn");
    let scroll = |fx: &mut Fx, dy: f32| {
        fx.vcx.simulate_event(ScrollWheelEvent {
            position: bounds.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        fx.settle();
    };
    scroll(&mut fx, -2_000.);
    assert_eq!(fx.model(|m| m.tail()), Tail::Paused { auto: true });
    fx.tick();
    assert_eq!(fx.uids()[0], "e299", "rows do not move under the pointer");
    scroll(&mut fx, 4_000.);
    assert_eq!(fx.model(|m| m.tail()), Tail::Live);
    assert_eq!(fx.uids()[0], "late");
}

#[gpui::test]
fn the_status_line_counts_the_rows(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.open();
    assert!(fx.drawn("events-status"));
    let view = fx.view().unwrap();
    let state = fx.vcx.update(|_, cx| view.read(cx).feed_state().clone());
    assert_eq!(state, oxikube_app::events::EventsState::Ready);
}
