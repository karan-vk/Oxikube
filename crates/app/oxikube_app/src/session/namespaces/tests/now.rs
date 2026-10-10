//! Selecting now (E05-P600): the session changes on the caller's thread, the remembering runs
//! later and keeps the last selection applied whatever order it runs in.

use oxikube_domain::command::Command;
use oxikube_domain::session::NamespaceSelection;
use oxikube_testkit::StateCall;

use super::*;
use crate::session::namespaces::DEBOUNCE;

fn set(names: &[&str]) -> NamespaceSelection {
    NamespaceSelection::from_names(names)
}

fn kv_writes(h: &Harness) -> usize {
    h.state
        .recorded_calls()
        .iter()
        .filter(|c| matches!(c, StateCall::KvSet(..)))
        .count()
}

#[test]
fn select_now_changes_the_session_before_anything_is_awaited() {
    let mut h = Harness::new();
    h.connect("a", &["x", "y"]);
    h.namespace_changes();
    let writes = kv_writes(&h);

    let now = h.service.select_now(&id("a"), set(&["x"])).unwrap();

    // No executor has run: the session and its update are already there, the write is not.
    assert!(now.session_changed);
    assert_eq!(h.selection("a"), set(&["x"]));
    assert_eq!(h.namespace_changes(), vec![(id("a"), set(&["x"]))]);
    assert_eq!(kv_writes(&h), writes, "nothing written yet");

    let outcome = h.run(now.remember()).unwrap();
    assert!(outcome.changed);
    assert_eq!(outcome.prefs.selection, set(&["x"]));
    assert_eq!(kv_writes(&h), writes + 1);
    // A restart reads what was remembered.
    assert_eq!(
        h.run(h.restarted().prefs(&id("a"))).unwrap().selection,
        set(&["x"])
    );
}

#[test]
fn rememberings_that_finish_out_of_order_keep_the_last_selection() {
    let h = Harness::new();
    h.connect("a", &["x", "y"]);
    let first = h.service.select_now(&id("a"), set(&["x"])).unwrap();
    let second = h.service.select_now(&id("a"), set(&["y"])).unwrap();

    // The later one is written first; the earlier one must not overwrite it.
    h.run(second.remember()).unwrap();
    h.run(first.remember()).unwrap();

    assert_eq!(h.selection("a"), set(&["y"]));
    assert_eq!(
        h.run(h.service.prefs(&id("a"))).unwrap().selection,
        set(&["y"])
    );
    assert_eq!(
        h.run(h.restarted().prefs(&id("a"))).unwrap().selection,
        set(&["y"]),
        "the stored record is the last selection"
    );
}

#[test]
fn select_now_cancels_a_pending_debounced_tick() {
    let h = Harness::new();
    h.connect("a", &["x", "y"]);
    let (service, cluster) = (h.service.clone(), id("a"));
    let mut tick = Box::pin(async move { service.select_debounced(&cluster, set(&["x"])).await });
    assert!(futures::FutureExt::now_or_never(&mut tick).is_none());

    let now = h.service.select_now(&id("a"), set(&["y"])).unwrap();
    h.run(now.remember()).unwrap();
    h.clock.advance(DEBOUNCE);

    assert_eq!(h.run(tick).unwrap(), None, "the tick was superseded");
    assert_eq!(h.selection("a"), set(&["y"]));
}

#[test]
fn execute_now_runs_namespace_select_only() {
    let h = Harness::new();
    h.connect("a", &["x"]);
    let select = Command::NamespaceSelect {
        cluster: id("a"),
        namespaces: vec!["x".into()],
    };
    let now = h.service.execute_now(&select).unwrap();
    assert_eq!(h.selection("a"), set(&["x"]));
    h.run(now.remember()).unwrap();

    let favourite = Command::NamespaceToggleFavourite {
        cluster: id("a"),
        namespace: "x".into(),
    };
    let err = h.service.execute_now(&favourite).unwrap_err();
    assert_eq!(err.kind(), oxikube_domain::ErrorKind::Unsupported);
}

#[test]
fn select_now_on_a_session_that_is_not_open_is_not_found() {
    let h = Harness::new();
    let err = h.service.select_now(&id("zzz"), set(&["x"])).unwrap_err();
    assert_eq!(err.kind(), oxikube_domain::ErrorKind::NotFound);
}
