//! Out-of-order and duplicate deltas converge without panicking.

use oxikube_ports::Delta;

use super::*;

#[test]
fn modify_before_add_delete_of_unknown_and_duplicates_converge() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![])]),
        batch(vec![
            Delta::Applied(p("x", "b", "2")), // a modify of an object never added
            Delta::Deleted(p("x", "ghost", "1")), // a delete of an unknown object
            Delta::Applied(p("x", "a", "1")),
            Delta::Applied(p("x", "a", "1")), // duplicate
        ]),
        batch(vec![
            Delta::Applied(p("x", "a", "1")), // the same version again: no op
            Delta::Deleted(p("x", "b", "1")),
            Delta::Deleted(p("x", "b", "1")), // duplicate delete
            Delta::Applied(p("x", "c", "1")),
            Delta::Deleted(p("x", "c", "1")), // added and deleted in one batch
        ]),
    ]));
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    h.advance(1);
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/a", "x/b"]);
    h.advance(1);
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/a"]);
    assert_eq!(h.store.feeds()[0].objects, 1);
}

#[test]
fn a_restart_inside_a_batch_discards_what_came_before_it() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![batch(vec![
        Delta::Applied(p("x", "early", "1")),
        Delta::Restarted(vec![p("x", "a", "1")]),
        Delta::Applied(p("x", "b", "1")),
    ])]));
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/a", "x/b"]);
}

#[test]
fn an_unchanged_relist_sends_nothing_but_the_state() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![p("x", "a", "1")])]),
        batch(vec![Delta::Restarted(vec![p("x", "a", "1")])]),
    ]));
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    h.advance(1);
    assert_eq!(
        m.drain(&mut sub),
        0,
        "same versions: nothing to tell the view"
    );
    assert_eq!(m.names(), ["x/a"]);
}
