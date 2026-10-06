//! `connect`: who connects at launch, in what order, how many at once.

use std::pin::pin;

use super::*;
use crate::session::restore::RestoreConnect;

fn prepared(
    h: &Harness,
    open: &[&str],
    active: Option<&str>,
) -> crate::session::restore::RestorePlan {
    h.save(open, active);
    block_on(h.restorer.prepare()).expect("prepare")
}

#[test]
fn only_the_displayed_cluster_connects_by_default() {
    let h = Harness::new();
    let plan = prepared(&h, &["a", "b", "c"], Some("b"));

    let report = block_on(h.restorer.connect(&plan, RestoreConnect::Active));

    assert_eq!(
        h.connects(),
        ["b"],
        "the others wait for their tab to be shown"
    );
    assert_eq!(h.phase("b"), SessionPhase::Ready);
    assert_eq!(h.phase("a"), SessionPhase::Disconnected);
    assert_eq!(h.phase("c"), SessionPhase::Disconnected);
    assert_eq!(report.outcomes.len(), 1);
    assert!(report.outcome(&id("b")).is_some_and(|o| o.attempted));
}

#[test]
fn with_the_catalog_displayed_nothing_connects() {
    let h = Harness::new();
    let plan = prepared(&h, &["a", "b"], None);
    let report = block_on(h.restorer.connect(&plan, RestoreConnect::Active));
    assert!(h.connects().is_empty());
    assert!(report.outcomes.is_empty());
}

#[test]
fn connect_all_starts_with_the_displayed_cluster_then_follows_tab_order() {
    let h = Harness::with_config(RestoreConfig {
        concurrency: 1,
        ..RestoreConfig::default()
    });
    let plan = prepared(&h, &["a", "b", "c"], Some("c"));

    let report = block_on(h.restorer.connect(&plan, RestoreConnect::All));

    assert_eq!(h.connects(), ["c", "a", "b"]);
    assert_eq!(report.outcomes.len(), 3);
    for name in ["a", "b", "c"] {
        assert_eq!(h.phase(name), SessionPhase::Ready, "{name}");
    }
}

#[test]
fn connect_all_runs_at_most_the_configured_number_at_once() {
    let h = Harness::new(); // concurrency 2
    let plan = prepared(&h, &["a", "b", "c"], Some("a"));
    h.connector.hold();

    let mut fut = pin!(h.restorer.connect(&plan, RestoreConnect::All));
    assert!(!poll_once(&mut fut));
    assert_eq!(
        h.connector.held(),
        2,
        "two attempts in flight, the third waits"
    );
    assert_eq!(h.connects(), ["a", "b"]);

    h.connector.release();
    block_on(fut);

    assert_eq!(h.connects(), ["a", "b", "c"]);
    for name in ["a", "b", "c"] {
        assert_eq!(h.phase(name), SessionPhase::Ready, "{name}");
    }
}

#[test]
fn a_cluster_the_user_already_connected_is_left_alone() {
    let h = Harness::new();
    let plan = prepared(&h, &["a", "b"], Some("a"));
    block_on(h.sessions.connect(&id("a"))).expect("connect");
    h.connector.clear_calls();

    let report = block_on(h.restorer.connect(&plan, RestoreConnect::All));

    assert_eq!(h.connects(), ["b"]);
    let a = report.outcome(&id("a")).expect("outcome");
    assert!(!a.attempted);
    assert_eq!(a.state.phase(), SessionPhase::Ready);
}

#[test]
fn clusters_whose_credentials_may_prompt_connect_one_at_a_time() {
    use oxikube_ports::ExecInteractivity;

    let h = Harness::new(); // concurrency 2
    let plan = prepared(&h, &["a", "b", "c"], Some("a"));
    // `a` and `b` may open an exec plugin prompt; `c` may not.
    for name in ["a", "b"] {
        h.sessions
            .set_exec_interactivity(&id(name), ExecInteractivity::Always)
            .expect("session");
    }
    h.connector.hold();

    let mut fut = pin!(h.restorer.connect(&plan, RestoreConnect::All));
    assert!(!poll_once(&mut fut));
    // `a` holds the prompt; `b` waits for it; `c` has a free slot and does not prompt.
    assert_eq!(h.connects(), ["a", "c"]);
    assert_eq!(h.connector.held(), 2);

    h.connector.release();
    block_on(fut);
    assert_eq!(h.connects(), ["a", "c", "b"]);
    assert!(
        ["a", "b", "c"]
            .iter()
            .all(|n| h.phase(n) == SessionPhase::Ready)
    );
}

#[test]
fn a_cluster_dismissed_while_queued_is_never_connected() {
    let h = Harness::with_config(RestoreConfig {
        concurrency: 1,
        ..RestoreConfig::default()
    });
    let plan = prepared(&h, &["a", "b", "c"], Some("a"));
    h.connector.hold();

    // `a` holds the only slot, `b` and `c` wait for it.
    let mut fut = pin!(h.restorer.connect(&plan, RestoreConnect::All));
    assert!(!poll_once(&mut fut));
    assert_eq!(h.connects(), ["a"]);

    // The user closes the placeholder of `b` while it still waits.
    h.restorer.skips().skip(&id("b"));
    h.connector.release();
    let report = block_on(fut);

    assert_eq!(h.connects(), ["a", "c"], "b was never connected");
    assert_eq!(h.phase("b"), SessionPhase::Disconnected);
    assert_eq!(h.phase("c"), SessionPhase::Ready);
    let b = report.outcome(&id("b")).expect("outcome");
    assert!(!b.attempted && !b.failed());
}
