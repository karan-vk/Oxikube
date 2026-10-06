//! Per-cluster settings pushed into the manager (E06-S08): new sessions start from them,
//! open sessions follow them live, and only the clusters that changed hear about it.

use oxikube_domain::ClusterColour;
use oxikube_domain::session::{NamespaceSelection, SessionPhase};
use oxikube_ports::{ClusterPrefs, ClusterPrefsTable, ExecInteractivity};
use oxikube_testkit::ConnectorCall;

use super::{Harness, ctx, id};
use crate::session::{SessionChange, SessionOptions};

const RED: ClusterColour = ClusterColour::rgb(0xe5, 0x48, 0x4d);

fn prod_prefs() -> ClusterPrefs {
    ClusterPrefs {
        display_name: Some("Production".into()),
        colour: Some(RED),
        read_only: true,
        default_namespace: Some("payments".into()),
        exec_interactivity: ExecInteractivity::IfAvailable,
        accessible_namespaces: vec!["payments".into(), "billing".into()],
        ..ClusterPrefs::default()
    }
}

fn table(prefs: ClusterPrefs) -> ClusterPrefsTable {
    ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(id("a"), prefs)
}

fn changes_of(h: &mut Harness, name: &str) -> Vec<SessionChange> {
    let cluster = id(name);
    h.drain()
        .into_iter()
        .filter(|u| u.cluster == cluster)
        .map(|u| u.change)
        .collect()
}

#[test]
fn a_session_opened_from_the_catalog_starts_from_its_settings() {
    let h = Harness::new();
    h.manager.set_prefs_table(table(prod_prefs()));

    // Not opened first: `connect` opens it from the catalog. A read-only production cluster
    // must never come up writable.
    h.connect("a");

    let a = h.manager.get(&id("a")).unwrap();
    assert!(a.read_only());
    assert_eq!(a.colour(), Some(RED));
    assert_eq!(a.display_name(), Some("Production"));
    assert_eq!(a.title(), "Production");
    assert_eq!(
        a.namespace_selection(),
        &NamespaceSelection::single("payments")
    );
    assert_eq!(a.exec_interactivity(), ExecInteractivity::IfAvailable);
    assert_eq!(a.prefs().accessible_namespaces, ["payments", "billing"]);

    let b = h.manager.open_configured(&ctx("b"));
    assert!(!b.read_only());
    assert_eq!(b.title(), "b", "no display name: the context name");
    assert!(b.namespace_selection().is_all());
}

#[test]
fn the_kubeconfig_namespace_is_the_fallback_default_namespace() {
    let h = Harness::new();
    let mut context = ctx("b");
    context.default_namespace = Some("kube-ns".into());
    let session = h.manager.open_configured(&context);
    assert_eq!(
        session.namespace_selection(),
        &NamespaceSelection::single("kube-ns")
    );

    // The cluster's own setting wins over the kubeconfig's.
    h.manager.set_prefs_table(table(prod_prefs()));
    let mut a = ctx("a");
    a.default_namespace = Some("kube-ns".into());
    let session = h.manager.open_configured(&a);
    assert_eq!(
        session.namespace_selection(),
        &NamespaceSelection::single("payments")
    );
}

#[test]
fn open_sessions_follow_a_new_colour_and_read_only_flag_without_reconnecting() {
    let mut h = Harness::new();
    h.manager.open_configured(&ctx("a"));
    assert_eq!(h.connect("a").phase(), SessionPhase::Ready);
    h.drain();
    let connects = h.connector.recorded_calls().len();

    let changed = h.manager.set_prefs_table(table(ClusterPrefs {
        colour: Some(RED),
        read_only: true,
        ..ClusterPrefs::default()
    }));

    assert_eq!(changed, 1);
    let a = h.manager.get(&id("a")).unwrap();
    assert_eq!((a.read_only(), a.colour()), (true, Some(RED)));
    assert_eq!(a.phase(), SessionPhase::Ready, "still connected");
    assert_eq!(h.connector.recorded_calls().len(), connects, "no reconnect");
    assert_eq!(h.connector.live_connections(&id("a")), 1);
    assert_eq!(
        changes_of(&mut h, "a"),
        [
            SessionChange::ReadOnlyChanged(true),
            SessionChange::ColourChanged(Some(RED))
        ]
    );
}

#[test]
fn display_name_changes_are_announced() {
    let mut h = Harness::new();
    h.manager.open_configured(&ctx("a"));
    h.drain();
    h.manager.set_prefs_table(table(prod_prefs()));
    let changes = changes_of(&mut h, "a");
    assert!(changes.contains(&SessionChange::DisplayNameChanged(Some(
        "Production".into()
    ))));

    h.manager.set_prefs_table(table(ClusterPrefs::default()));
    assert_eq!(
        changes_of(&mut h, "a").last(),
        Some(&SessionChange::DisplayNameChanged(None))
    );
    assert_eq!(h.manager.get(&id("a")).unwrap().title(), "a");
}

#[test]
fn only_the_cluster_that_changed_is_touched_or_announced() {
    let mut h = Harness::new();
    h.manager.open_configured(&ctx("a"));
    h.manager.open_configured(&ctx("b"));
    // A manual change on b (an in-app toggle that has not reached the file yet).
    h.manager.set_read_only(&id("b"), true).unwrap();
    h.drain();

    let changed = h.manager.set_prefs_table(table(prod_prefs()));

    assert_eq!(
        changed, 1,
        "b has no prefs of its own and they did not change"
    );
    assert!(
        h.manager.get(&id("b")).unwrap().read_only(),
        "b is left as it was"
    );
    let updates = h.drain();
    assert!(updates.iter().all(|u| u.cluster == id("a")), "{updates:?}");
    assert!(!updates.is_empty());

    // The same table again changes nothing and announces nothing.
    assert_eq!(h.manager.set_prefs_table(table(prod_prefs())), 0);
    assert!(h.drain().is_empty());
}

#[test]
fn a_global_change_reaches_every_cluster_without_its_own_block() {
    let mut h = Harness::new();
    h.manager.open_configured(&ctx("a"));
    h.manager.open_configured(&ctx("b"));
    h.drain();
    let all_read_only = ClusterPrefs {
        read_only: true,
        ..ClusterPrefs::default()
    };
    let changed = h
        .manager
        .set_prefs_table(ClusterPrefsTable::new(all_read_only));
    assert_eq!(changed, 2);
    assert!(h.manager.sessions().iter().all(|s| s.read_only()));
}

#[test]
fn the_exec_policy_applies_on_the_next_connect_not_to_the_current_one() {
    let h = Harness::new();
    h.manager.open_configured(&ctx("a"));
    h.connect("a");
    h.manager.set_prefs_table(table(ClusterPrefs {
        exec_interactivity: ExecInteractivity::Always,
        ..ClusterPrefs::default()
    }));
    assert_eq!(h.connector.live_connections(&id("a")), 1, "not reconnected");

    h.reconnect("a");

    let calls = h.connector.recorded_calls();
    let ConnectorCall::Connect {
        exec_interactivity, ..
    } = calls.last().unwrap();
    assert_eq!(*exec_interactivity, ExecInteractivity::Always);
}

#[test]
fn explicit_options_still_open_a_session_as_asked() {
    let h = Harness::new();
    h.manager.set_prefs_table(table(prod_prefs()));
    let session = h.manager.open(
        &ctx("a"),
        SessionOptions {
            read_only: false,
            ..SessionOptions::default()
        },
    );
    assert!(
        !session.read_only(),
        "`open` takes the caller's options as given"
    );
    // ...until the next push says otherwise.
    h.manager.set_prefs_table(table(ClusterPrefs {
        read_only: true,
        ..prod_prefs()
    }));
    assert!(h.manager.get(&id("a")).unwrap().read_only());
}
