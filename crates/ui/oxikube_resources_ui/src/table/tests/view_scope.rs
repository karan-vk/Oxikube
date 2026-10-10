//! A namespace change made on the UI thread shows in that update (E05-P600): the session echo
//! rescopes the table, which keeps the held rows of the new scope at once and lets the store's
//! snapshot reconcile them later.

use gpui::{Entity, TestAppContext};
use oxikube_app::store::FeedState;
use oxikube_domain::Resource;
use oxikube_domain::session::{NamespaceSelection, WatchScope};
use oxikube_testkit::pod;
use oxikube_workspace::cluster::SessionEcho;

use super::fixture::{Fixture, cluster};
use crate::table::ResourceTable;
use crate::table::scope::covers;

fn p(namespace: &str, name: &str) -> Resource {
    pod().namespace(namespace).name(name).build()
}

/// Changes the selection as an immediate command does: on the UI thread, inside an echo, with no
/// executor turn afterwards.
fn select_now(f: &mut Fixture, selection: NamespaceSelection) {
    let sessions = f.sessions.clone();
    f.vcx.update(|_, cx| {
        let echo = SessionEcho::begin(&sessions);
        sessions
            .set_namespace_selection(&cluster(), selection)
            .expect("open session");
        echo.finish(cx);
    });
}

fn state(f: &mut Fixture, table: &Entity<ResourceTable>) -> FeedState {
    f.vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.state().clone()))
}

fn columns(f: &mut Fixture, table: &Entity<ResourceTable>) -> Vec<String> {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .columns()
                .filter(|(_, shown)| *shown)
                .map(|(c, _)| c.id.to_string())
                .collect()
        })
    })
}

fn three_pods(cx: &mut TestAppContext) -> (Fixture, Entity<ResourceTable>) {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "a"), p("x", "b"), p("y", "c")]);
    let table = f.open_pods();
    assert_eq!(f.names(&table), ["a", "b", "c"]);
    assert_eq!(state(&mut f, &table), FeedState::Ready);
    (f, table)
}

#[gpui::test]
fn narrowing_shows_the_held_rows_in_the_same_update(cx: &mut TestAppContext) {
    let (mut f, table) = three_pods(cx);

    assert!(columns(&mut f, &table).contains(&"namespace".to_owned()));

    select_now(&mut f, NamespaceSelection::single("x"));

    assert_eq!(
        f.names(&table),
        ["a", "b"],
        "narrowed before any executor turn"
    );
    assert!(
        !columns(&mut f, &table).contains(&"namespace".to_owned()),
        "the columns follow in the same frame: one namespace hides the Namespace column"
    );
    assert_eq!(
        state(&mut f, &table),
        FeedState::Ready,
        "the held rows are the whole new list"
    );
    // The store's reseed agrees.
    f.settle();
    assert_eq!(f.names(&table), ["a", "b"]);
    assert_eq!(state(&mut f, &table), FeedState::Ready);
}

#[gpui::test]
fn moving_to_another_namespace_shows_loading_until_the_snapshot(cx: &mut TestAppContext) {
    let (mut f, table) = three_pods(cx);
    select_now(&mut f, NamespaceSelection::single("x"));
    f.settle();

    select_now(&mut f, NamespaceSelection::single("y"));

    assert!(f.names(&table).is_empty(), "nothing of `y` was held");
    assert_eq!(
        state(&mut f, &table),
        FeedState::Warming,
        "shown as loading"
    );
    f.settle();
    assert_eq!(f.names(&table), ["c"]);
    assert_eq!(state(&mut f, &table), FeedState::Ready);

    // Back to all: `c` stays on screen, marked as refreshing, until the rest arrive.
    select_now(&mut f, NamespaceSelection::All);
    assert_eq!(f.names(&table), ["c"]);
    assert_eq!(state(&mut f, &table), FeedState::Warming);
    f.settle();
    assert_eq!(f.names(&table), ["a", "b", "c"]);
}

#[gpui::test]
fn the_selection_keeps_the_rows_that_stay(cx: &mut TestAppContext) {
    let (mut f, table) = three_pods(cx);
    f.keys(&table, "j shift-j shift-j");
    assert_eq!(f.selected(&table), ["a", "b", "c"]);

    select_now(&mut f, NamespaceSelection::single("y"));
    assert_eq!(
        f.selected(&table),
        ["c"],
        "the rows that left are unselected"
    );
}

#[test]
fn held_rows_cover_a_subset_only() {
    let ns = |names: &[&str]| WatchScope::Namespaces(names.iter().map(|n| (*n).into()).collect());
    assert!(covers(&WatchScope::Cluster, &ns(&["x"])));
    assert!(covers(&ns(&["x", "y"]), &ns(&["y"])));
    assert!(!covers(&ns(&["x"]), &ns(&["x", "y"])));
    assert!(!covers(&ns(&["x"]), &ns(&["y"])));
    assert!(!covers(&ns(&["x"]), &WatchScope::Cluster));
}
