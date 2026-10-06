//! Rapid toggles re-scope the feeds once.

use std::time::Duration;

use futures::future::{join_all, poll_immediate};
use futures::pin_mut;
use oxikube_domain::session::NamespaceSelection;

use super::*;

fn set(names: &[&str]) -> NamespaceSelection {
    NamespaceSelection::from_names(names)
}

#[test]
fn ticking_five_boxes_sends_one_change() {
    let mut h = Harness::new();
    h.connect("a", &["a", "b", "c", "d", "e"]);
    h.namespace_changes();
    let cluster = id("a");
    let service = h.service.clone();
    let toggles = ["a", "b", "c", "d", "e"].iter().enumerate().map(|(i, _)| {
        let service = service.clone();
        let cluster = cluster.clone();
        let names = ["a", "b", "c", "d", "e"][..=i].to_vec();
        async move { service.select_debounced(&cluster, set(&names)).await }
    });
    let all = join_all(toggles);
    pin_mut!(all);

    // Every toggle waits for its quiet time; nothing is applied yet.
    assert!(futures::executor::block_on(poll_immediate(&mut all)).is_none());
    assert_eq!(h.selection("a"), NamespaceSelection::All);
    assert_eq!(h.clock.pending_sleepers(), 5);

    h.clock.advance(Duration::from_millis(150));
    let results = futures::executor::block_on(all);

    let applied: Vec<_> = results.into_iter().map(|r| r.unwrap()).collect();
    assert_eq!(
        applied.iter().filter(|r| r.is_some()).count(),
        1,
        "only the last applies"
    );
    assert!(applied.last().unwrap().is_some());
    assert_eq!(
        h.namespace_changes(),
        vec![(id("a"), set(&["a", "b", "c", "d", "e"]))]
    );
}

#[test]
fn nothing_is_applied_before_the_quiet_time_ends() {
    let mut h = Harness::new();
    h.connect("a", &["x"]);
    h.namespace_changes();
    let (service, a) = (h.service.clone(), id("a"));
    let fut = service.select_debounced(&a, set(&["x"]));
    pin_mut!(fut);

    assert!(futures::executor::block_on(poll_immediate(&mut fut)).is_none());
    h.clock.advance(Duration::from_millis(149));
    assert!(futures::executor::block_on(poll_immediate(&mut fut)).is_none());
    assert!(h.namespace_changes().is_empty());

    h.clock.advance(Duration::from_millis(1));
    let outcome = futures::executor::block_on(fut).unwrap().expect("applied");
    assert!(outcome.changed);
    assert_eq!(h.namespace_changes(), vec![(id("a"), set(&["x"]))]);
}

#[test]
fn an_immediate_select_cancels_a_pending_toggle() {
    let mut h = Harness::new();
    h.connect("a", &["x", "y"]);
    h.namespace_changes();
    let (service, a) = (h.service.clone(), id("a"));
    let pending = service.select_debounced(&a, set(&["x"]));
    pin_mut!(pending);
    assert!(futures::executor::block_on(poll_immediate(&mut pending)).is_none());

    h.run(h.service.select(&id("a"), set(&["y"]))).unwrap();
    h.clock.advance(Duration::from_millis(150));

    assert!(
        futures::executor::block_on(pending).unwrap().is_none(),
        "superseded"
    );
    assert_eq!(h.selection("a"), set(&["y"]));
    assert_eq!(h.namespace_changes(), vec![(id("a"), set(&["y"]))]);
}

#[test]
fn dropping_a_pending_toggle_cancels_it() {
    let mut h = Harness::new();
    h.connect("a", &["x"]);
    h.namespace_changes();
    {
        let (service, a) = (h.service.clone(), id("a"));
        let pending = service.select_debounced(&a, set(&["x"]));
        pin_mut!(pending);
        assert!(futures::executor::block_on(poll_immediate(&mut pending)).is_none());
    }

    h.clock.advance(Duration::from_millis(500));

    assert!(h.namespace_changes().is_empty());
    assert_eq!(h.selection("a"), NamespaceSelection::All);
}

#[test]
fn clusters_debounce_independently() {
    let mut h = Harness::new();
    h.connect("a", &["x"]);
    h.connect("b", &["y"]);
    h.namespace_changes();
    let (service, a, b) = (h.service.clone(), id("a"), id("b"));
    let both = futures::future::join(
        service.select_debounced(&a, set(&["x"])),
        service.select_debounced(&b, set(&["y"])),
    );
    pin_mut!(both);
    assert!(futures::executor::block_on(poll_immediate(&mut both)).is_none());

    h.clock.advance(Duration::from_millis(150));
    let (a, b) = futures::executor::block_on(both);

    assert!(a.unwrap().is_some() && b.unwrap().is_some());
    assert_eq!(h.namespace_changes().len(), 2);
}
