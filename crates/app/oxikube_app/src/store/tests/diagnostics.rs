//! What a table needs to tell loading from forbidden from expired credentials, to retry, and to
//! show the server's warnings once (E07-S10).

use std::time::Duration;

use futures::{FutureExt as _, StreamExt as _};
use oxikube_domain::OxiError;
use oxikube_ports::{ApiWarning, Delta};
use oxikube_testkit::{ResourceCall, Timeline};

use super::*;
use crate::store::FeedState;

fn watches(h: &Harness) -> usize {
    h.resources
        .recorded_calls()
        .iter()
        .filter(|c| matches!(c, ResourceCall::Watch { .. }))
        .count()
}

#[test]
fn expired_credentials_are_unauthorized_not_forbidden() {
    let mut h = Harness::new();
    h.resources
        .script()
        .watch
        .push_err(OxiError::auth("the token has expired", false));
    let sub = h.subscribe(all(pods()));
    assert!(
        matches!(sub.state(), FeedState::Unauthorized { message } if message.contains("expired")),
        "{:?}",
        sub.state()
    );
    assert!(sub.state().is_terminal());
}

#[test]
fn retry_restarts_a_stopped_feed_and_keeps_the_rows_until_the_new_list() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::ZERO,
                batch(vec![Delta::Restarted(vec![p("x", "a", "1")])]),
            )
            .err_at(
                Duration::from_secs(1),
                OxiError::forbidden("pods is forbidden"),
            ),
    );
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    h.advance(1);
    m.drain(&mut sub);
    assert!(matches!(
        m.last.as_ref().unwrap().state,
        FeedState::Forbidden { .. }
    ));
    assert_eq!(m.names(), ["x/a"], "the rows stay under the error");
    assert_eq!(watches(&h), 1);

    // The second attempt lists a newer object; until it arrives the old rows are still there.
    h.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::from_secs(5),
                batch(vec![Delta::Restarted(vec![
                    p("x", "a", "2"),
                    p("x", "b", "1"),
                ])]),
            )
            .keep_open(),
    );
    sub.retry();
    h.settle();
    assert_eq!(watches(&h), 2, "retry reopened the feed");
    m.drain(&mut sub);
    assert_eq!(sub.state(), FeedState::Warming);
    assert_eq!(m.names(), ["x/a"], "stale rows stay while it warms");
    h.advance(5);
    m.drain(&mut sub);
    assert_eq!(m.last.as_ref().unwrap().state, FeedState::Ready);
    assert_eq!(m.names(), ["x/a", "x/b"], "recovery shows the new rows");
}

#[test]
fn retry_of_an_unauthorized_feed_recovers_once_the_credentials_work() {
    let mut h = Harness::new();
    h.resources
        .script()
        .watch
        .push_err(OxiError::auth("token expired", false));
    let mut sub = h.subscribe(all(pods()));
    assert!(matches!(sub.state(), FeedState::Unauthorized { .. }));
    h.resources.insert(p("x", "a", "1"));
    sub.retry();
    h.settle();
    assert_eq!(sub.state(), FeedState::Ready);
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/a"]);
}

#[test]
fn retry_cuts_a_backoff_short_and_does_nothing_for_a_ready_feed() {
    let mut h = Harness::new();
    h.resources
        .script()
        .watch
        .push_err(OxiError::network("connection refused"));
    let mut sub = h.subscribe(all(pods()));
    assert!(matches!(sub.state(), FeedState::Retrying { .. }));
    assert_eq!(watches(&h), 1);
    // No time passes: the backoff is not over, but the user asked.
    sub.retry();
    h.settle();
    assert_eq!(watches(&h), 2, "reopened at once");
    assert_eq!(sub.state(), FeedState::Ready);

    sub.retry();
    h.settle();
    assert_eq!(watches(&h), 2, "a ready feed is left alone");
}

#[test]
fn warnings_are_shown_once_per_distinct_text_however_many_views_ask() {
    let mut h = Harness::new();
    let mut first = h.store.warnings();
    let mut second = h.store.warnings();
    h.warnings.push_text("v1 Endpoints is deprecated");
    h.warnings.push_text("v1 Endpoints is deprecated");
    let mut seen = Vec::new();
    for stream in [&mut first, &mut second] {
        while let Some(w) = stream.next().now_or_never().flatten() {
            seen.push(w);
        }
    }
    assert_eq!(
        seen,
        vec![ApiWarning::new("v1 Endpoints is deprecated")],
        "one toast for the repeat, across both views"
    );

    h.warnings.push_text("unknown field spec.foo");
    let next = first.next().now_or_never().flatten();
    assert_eq!(next, Some(ApiWarning::new("unknown field spec.foo")));
    h.settle();
}

#[test]
fn a_store_without_a_warning_port_has_no_warnings() {
    use futures::executor::block_on;
    let clock = Arc::new(FakeClockPort::default());
    let store = ResourceStore::new(
        ClusterId::new("test", &ContextName::from("kind")),
        StorePorts {
            resources: Arc::new(FakeResourcePort::with_clock(clock.clone())),
            tables: Arc::new(FakeTableFeedPort::with_clock(clock.clone())),
        },
        StoreRuntime {
            spawner: Executor::default().spawner(),
            clock,
            probe: None,
        },
        StoreOptions::default(),
    );
    assert_eq!(block_on(store.warnings().next()), None);
}
