//! Ref-counted feeds: shared by equal queries, kept through the grace period, aborted after.

use oxikube_testkit::ResourceCall;

use super::*;
use crate::store::FeedState;

fn watch_calls(h: &Harness) -> usize {
    h.resources
        .recorded_calls()
        .iter()
        .filter(|c| matches!(c, ResourceCall::Watch { .. }))
        .count()
}

#[test]
fn two_subscribers_share_one_feed_and_the_last_drop_stops_it_after_the_grace() {
    let mut h = Harness::with_options(options_with_grace(10));
    h.resources.insert(p("x", "a", "1"));
    let mut first = h.subscribe(all(pods()));
    let mut second = h.subscribe(all(pods()).with_filter(crate::store::StoreFilter::text("a")));
    assert_eq!(watch_calls(&h), 1, "one feed for both");
    assert_eq!(h.resources.live_watches(), 1);
    let info = h.store.feeds();
    assert_eq!(info.len(), 1);
    assert_eq!(info[0].subscribers, 2);
    assert_eq!(info[0].objects, 1);
    assert_eq!(info[0].state, FeedState::Ready);

    let (mut a, mut b) = (Mirror::default(), Mirror::default());
    a.drain(&mut first);
    b.drain(&mut second);
    assert_eq!(a.names(), b.names());

    drop(first);
    h.advance(60);
    assert_eq!(h.resources.live_watches(), 1, "one subscriber left");
    assert_eq!(h.store.feeds()[0].subscribers, 1);

    drop(second);
    h.settle();
    assert_eq!(
        h.resources.live_watches(),
        1,
        "still running in its grace period"
    );
    assert!(h.store.feeds()[0].idle);
    h.advance(9);
    assert_eq!(h.resources.live_watches(), 1);
    h.advance(1);
    assert_eq!(
        h.resources.live_watches(),
        0,
        "aborted once the grace ran out"
    );
    assert!(h.store.feeds().is_empty());
    assert_eq!(h.clock.pending_sleepers(), 0, "no timer left behind");
}

#[test]
fn a_subscriber_within_the_grace_period_reuses_the_running_feed() {
    let mut h = Harness::with_options(options_with_grace(10));
    h.resources.insert(p("x", "a", "1"));
    drop(h.subscribe(all(pods())));
    h.advance(5);
    let mut again = h.subscribe(all(pods()));
    assert_eq!(watch_calls(&h), 1, "no relist on a tab switch");
    let mut m = Mirror::default();
    m.drain(&mut again);
    assert_eq!(m.names(), ["x/a"], "served from the cache at once");
    assert_eq!(m.last.as_ref().unwrap().state, FeedState::Ready);

    h.advance(30);
    assert_eq!(
        h.resources.live_watches(),
        1,
        "the old grace timer was cancelled"
    );
    drop(again);
    h.advance(10);
    assert_eq!(h.resources.live_watches(), 0);
}

#[test]
fn zero_grace_stops_the_feed_on_the_last_drop() {
    let mut h = Harness::with_options(options_with_grace(0));
    let sub = h.subscribe(all(pods()));
    assert_eq!(h.resources.live_watches(), 1);
    drop(sub);
    h.settle();
    assert_eq!(h.resources.live_watches(), 0);
    assert!(h.store.feeds().is_empty());
}

#[test]
fn different_scopes_are_different_feeds() {
    let mut h = Harness::new();
    let _all = h.subscribe(all(pods()));
    let _one = h.subscribe(in_namespaces(pods(), &["x"]));
    let _nodes = h.subscribe(all(oxikube_domain::ids::Gvk::new("", "v1", "Node")));
    assert_eq!(watch_calls(&h), 3);
    assert_eq!(h.resources.live_watches(), 3);
    assert_eq!(h.store.feeds().len(), 3);
}
