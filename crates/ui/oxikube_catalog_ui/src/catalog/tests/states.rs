//! Connection states shown on the rows come from the session manager, whoever changes them.

use gpui::TestAppContext;
use oxikube_domain::session::ClusterSessionState;

use super::{Fixture, Setup, contexts, id};
use crate::catalog::test_support::context;

fn connect(f: &mut Fixture, name: &str) {
    let manager = f.sessions.clone();
    let cluster = id(name);
    let task = f
        .app
        .cx()
        .executor()
        .spawn(async move { manager.connect(&cluster).await });
    f.app.run_until_parked();
    drop(task);
}

#[gpui::test]
fn a_session_changed_by_something_else_updates_the_badge(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(3));
    connect(&mut f, "ctx-02");
    let badge = f.read(|v| v.model().badge(2)).unwrap();
    assert_eq!(badge.label, "Connected");

    f.sessions.disconnect(&id("ctx-02")).unwrap();
    f.app.run_until_parked();
    assert_eq!(
        f.read(|v| v.model().badge(2)).unwrap().label,
        "Disconnected"
    );
}

#[gpui::test]
fn a_session_that_was_already_connected_when_the_view_opened_is_shown(cx: &mut TestAppContext) {
    let mut f = Fixture::start(cx, Setup::new(contexts(2)).connected(&["ctx-01"]));
    assert_eq!(
        f.read(|v| v.model().badge(0)).unwrap().label,
        "Disconnected"
    );
    assert_eq!(f.read(|v| v.model().badge(1)).unwrap().label, "Connected");
}

#[gpui::test]
fn a_burst_of_updates_for_a_cluster_not_in_the_catalog_changes_nothing_on_screen(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::open(cx, contexts(2));
    f.sessions.open(&context("elsewhere"), Default::default());
    f.app.run_until_parked();
    let states: Vec<_> = (0..2)
        .map(|ix| f.read(|v| v.model().badge(ix)).unwrap().label)
        .collect();
    assert_eq!(states, ["Disconnected", "Disconnected"]);
    assert_eq!(
        f.read(|v| v.model().state(&id("elsewhere")).clone()),
        ClusterSessionState::Disconnected
    );
}
