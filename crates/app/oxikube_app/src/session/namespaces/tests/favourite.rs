//! The number keys (E12-S16): `namespace::UseFavourite` selects All or the nth favourite, per
//! cluster, now when the favourites are loaded and from the state store otherwise. "Use this
//! namespace" (`u`) is `namespace::Select` with one name.

use oxikube_domain::ErrorKind;
use oxikube_domain::command::{Command, CommandId, lookup};
use oxikube_domain::session::NamespaceSelection;

use super::*;

fn use_favourite(cluster: &str, slot: u8) -> Command {
    Command::NamespaceUseFavourite {
        cluster: id(cluster),
        slot,
    }
}

fn pin(h: &Harness, cluster: &str, names: &[&str]) {
    for name in names {
        h.run(h.service.toggle_favourite(&id(cluster), name))
            .unwrap();
    }
}

#[test]
fn the_command_is_read_only_with_a_tool_name() {
    let meta = lookup(CommandId::NAMESPACE_USE_FAVOURITE).expect("registered");
    assert!(!meta.mutating && !meta.privileged);
    assert_eq!(
        CommandId::NAMESPACE_USE_FAVOURITE.tool_name(),
        "app.namespace_use_favourite"
    );
}

#[test]
fn a_digit_selects_the_nth_favourite_and_zero_selects_all() {
    let h = Harness::new();
    h.connect("a", &["dev", "prod", "stage"]);
    pin(&h, "a", &["prod", "dev", "stage"]);

    let outcome = h.run(h.service.execute(&use_favourite("a", 2))).unwrap();
    assert!(outcome.changed);
    assert_eq!(h.selection("a"), NamespaceSelection::single("dev"));
    h.run(h.service.execute(&use_favourite("a", 1))).unwrap();
    assert_eq!(h.selection("a"), NamespaceSelection::single("prod"));
    h.run(h.service.execute(&use_favourite("a", 0))).unwrap();
    assert_eq!(h.selection("a"), NamespaceSelection::All);
    // It is remembered like any other selection.
    h.run(h.service.execute(&use_favourite("a", 3))).unwrap();
    assert_eq!(
        h.run(h.restarted().prefs(&id("a"))).unwrap().selection,
        NamespaceSelection::single("stage")
    );
}

#[test]
fn a_slot_with_no_favourite_changes_nothing() {
    let h = Harness::new();
    h.connect("a", &["dev"]);
    pin(&h, "a", &["dev"]);
    h.run(h.service.execute(&use_favourite("a", 1))).unwrap();

    let outcome = h.run(h.service.execute(&use_favourite("a", 5))).unwrap();
    assert!(!outcome.changed);
    assert_eq!(h.selection("a"), NamespaceSelection::single("dev"));
}

#[test]
fn favourites_are_per_cluster() {
    let h = Harness::new();
    h.connect("a", &["dev"]);
    h.connect("b", &["other"]);
    pin(&h, "a", &["dev"]);
    pin(&h, "b", &["other"]);

    h.run(h.service.execute(&use_favourite("b", 1))).unwrap();
    assert_eq!(h.selection("b"), NamespaceSelection::single("other"));
    assert_eq!(
        h.selection("a"),
        NamespaceSelection::All,
        "cluster a keeps its own selection"
    );
}

#[test]
fn a_slot_above_nine_is_refused() {
    let h = Harness::new();
    h.connect("a", &["dev"]);
    let err = h
        .run(h.service.execute(&use_favourite("a", 10)))
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(h.service.use_favourite_now(&id("a"), 10).is_err());
}

#[test]
fn now_sets_the_session_in_the_calling_update_when_the_favourites_are_loaded() {
    let h = Harness::new();
    h.connect("a", &["dev", "prod"]);
    pin(&h, "a", &["prod", "dev"]);

    let now = h
        .service
        .use_favourite_now(&id("a"), 2)
        .unwrap()
        .expect("the favourites are loaded");
    assert!(now.session_changed);
    assert_eq!(h.selection("a"), NamespaceSelection::single("dev"));
    h.run(now.remember()).unwrap();
    assert_eq!(
        h.run(h.restarted().prefs(&id("a"))).unwrap().selection,
        NamespaceSelection::single("dev")
    );

    // Zero needs no favourites at all.
    let all = h.service.use_favourite_now(&id("a"), 0).unwrap().unwrap();
    assert!(all.session_changed);
    assert_eq!(h.selection("a"), NamespaceSelection::All);
}

#[test]
fn now_defers_when_it_cannot_say_here() {
    let h = Harness::new();
    h.connect("a", &["dev"]);
    pin(&h, "a", &["dev"]);

    // A second service has loaded nothing: the caller must run the async path.
    let fresh = h.restarted();
    assert!(fresh.use_favourite_now(&id("a"), 1).unwrap().is_none());
    h.run(fresh.use_favourite(&id("a"), 1)).unwrap();
    assert_eq!(h.selection("a"), NamespaceSelection::single("dev"));

    // Loaded, but no favourite at the slot: also deferred (the async path then does nothing).
    assert!(fresh.use_favourite_now(&id("a"), 7).unwrap().is_none());
}

#[test]
fn using_a_namespace_is_a_select_of_that_one_name() {
    let h = Harness::new();
    h.connect("a", &["dev", "prod"]);
    h.run(h.service.execute(&Command::NamespaceSelect {
        cluster: id("a"),
        namespaces: vec!["dev".into(), "prod".into()],
    }))
    .unwrap();

    h.run(h.service.execute(&Command::NamespaceSelect {
        cluster: id("a"),
        namespaces: vec!["prod".into()],
    }))
    .unwrap();

    assert_eq!(h.selection("a"), NamespaceSelection::single("prod"));
}
