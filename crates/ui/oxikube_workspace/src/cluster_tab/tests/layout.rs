//! Saved layout: which cluster tabs are open and in what order, and each cluster's own layout.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_testkit::StateCall;

use super::*;
use crate::{
    cluster_tab::{ClusterTabsStore, SavedTabs, cluster_layout_key},
    persistence::{LayoutStore, LoadOutcome, MAIN_WINDOW_ID, SAVE_DEBOUNCE},
};

fn settle(fx: &mut Fixture) {
    fx.vcx.run_until_parked();
    fx.vcx
        .executor()
        .advance_clock(SAVE_DEBOUNCE + Duration::from_millis(50));
    fx.vcx.run_until_parked();
}

fn saved_tabs(state: &Arc<FakeStatePort>) -> Option<SavedTabs> {
    let store = ClusterTabsStore::new(state.clone(), MAIN_WINDOW_ID).expect("store");
    block_on(store.load()).expect("load")
}

fn saved_layout(state: &Arc<FakeStatePort>, name: &str) -> LoadOutcome {
    let store = LayoutStore::new(state.clone(), &cluster_layout_key(&id(name))).expect("store");
    block_on(store.load()).expect("load")
}

fn open_item(fx: &mut Fixture, cluster: &str, title: &str) {
    let inner = fx.inner(cluster);
    let title = title.to_owned();
    fx.vcx.update(|window, cx| {
        let item = TestItem::build(title, cx);
        inner.update(cx, |ws, cx| ws.open_item(item, window, cx));
    });
}

fn titles_of(fx: &mut Fixture, cluster: &str) -> Vec<String> {
    let inner = fx.inner(cluster);
    let mut titles: Vec<String> = fx.vcx.update(|_, cx| {
        inner
            .read(cx)
            .items()
            .map(|item| item.tab_content(cx).title.to_string())
            .collect()
    });
    titles.sort();
    titles
}

#[gpui::test]
fn the_open_tabs_their_order_and_the_displayed_one_are_saved(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta", "gamma"]);
    assert_eq!(saved_tabs(&fx.state), None);
    for name in ["alpha", "beta", "gamma"] {
        fx.connect(name);
    }
    assert!(fx.apply(Command::ClusterSelect {
        cluster: id("beta")
    }));
    settle(&mut fx);

    let saved = saved_tabs(&fx.state).expect("saved");
    assert_eq!(saved.open, [id("alpha"), id("beta"), id("gamma")]);
    assert_eq!(saved.active, Some(id("beta")));
    assert_eq!(
        saved.title(&id("gamma")),
        Some("gamma"),
        "the names the tabs show are saved too, for the notice about a vanished cluster"
    );

    // Closing one updates the row.
    fx.disconnect("alpha");
    settle(&mut fx);
    let saved = saved_tabs(&fx.state).expect("saved");
    assert_eq!(saved.open, [id("beta"), id("gamma")]);
}

#[gpui::test]
fn each_clusters_layout_is_saved_under_its_own_key(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    open_item(&mut fx, "alpha", "pods");
    open_item(&mut fx, "alpha", "nodes");
    open_item(&mut fx, "beta", "events");
    settle(&mut fx);

    let LoadOutcome::Loaded(alpha) = saved_layout(&fx.state, "alpha") else {
        panic!("alpha's layout is saved");
    };
    let LoadOutcome::Loaded(beta) = saved_layout(&fx.state, "beta") else {
        panic!("beta's layout is saved");
    };
    assert_ne!(alpha.dock_area, beta.dock_area, "independent layouts");
    assert_eq!(
        alpha.window, None,
        "a cluster's workspace has no window of its own"
    );
    // The left dock (the sidebar) is part of each.
    assert!(alpha.dock_area.left_dock.is_some() && beta.dock_area.left_dock.is_some());
}

#[gpui::test]
fn reconnecting_a_cluster_gives_its_layout_back(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    open_item(&mut fx, "alpha", "pods");
    open_item(&mut fx, "alpha", "nodes");
    settle(&mut fx);

    // Closing the tab drops its workspace; opening it again restores what was in it.
    fx.disconnect("alpha");
    settle(&mut fx);
    assert_eq!(fx.open_names(), ["beta"]);
    fx.connect("alpha");
    assert_eq!(titles_of(&mut fx, "alpha"), ["nodes", "pods"]);
    assert_eq!(titles_of(&mut fx, "beta"), Vec::<String>::new());
}

#[gpui::test]
fn a_new_window_over_the_same_state_restores_a_clusters_layout(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    {
        let mut first = Fixture::open_with(cx, &["alpha"], state.clone(), true);
        first.connect("alpha");
        open_item(&mut first, "alpha", "pods");
        settle(&mut first);
    }
    // The next launch: nothing is open, session restore (E06-S11) connects the saved clusters.
    let mut second = Fixture::open_with(cx, &["alpha"], state.clone(), true);
    assert_eq!(second.open_names(), Vec::<String>::new());
    let saved = saved_tabs(&state).expect("the first run saved its tabs");
    assert_eq!(saved.open, [id("alpha")]);
    for cluster in &saved.open {
        block_on(second.sessions.connect(cluster)).expect("connect");
    }
    second.vcx.run_until_parked();
    assert_eq!(second.open_names(), ["alpha"]);
    assert_eq!(titles_of(&mut second, "alpha"), ["pods"]);
}

#[gpui::test]
fn nothing_is_written_until_something_happens(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha"]);
    settle(&mut fx);
    let puts = |fx: &Fixture| {
        fx.state
            .recorded_calls()
            .iter()
            .filter(|call| matches!(call, StateCall::TablePut(table, ..) if table.as_str() == "cluster_tabs"))
            .count()
    };
    // A window that starts empty must not overwrite what restore is about to read.
    assert_eq!(puts(&fx), 0);
    fx.connect("alpha");
    settle(&mut fx);
    assert_eq!(puts(&fx), 1);
    // A change that leaves the saved tabs as they are does not write again.
    fx.set_colour("alpha", Some(oxikube_domain::ClusterColour::rgb(1, 2, 3)));
    settle(&mut fx);
    assert_eq!(puts(&fx), 1);
}

#[gpui::test]
fn the_tabs_are_flushed_when_the_app_quits(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha"]);
    fx.connect("alpha");
    // Not settled: the debounce has not fired. A quit still saves.
    assert_eq!(saved_tabs(&fx.state), None);
    cx.update(|cx| cx.shutdown());
    cx.run_until_parked();
    let saved = saved_tabs(&fx.state).expect("flushed on quit");
    assert_eq!(saved.open, [id("alpha")]);
}

#[gpui::test]
fn unreadable_saved_tabs_read_as_nothing_saved(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    let store = ClusterTabsStore::new(state.clone(), MAIN_WINDOW_ID).expect("store");
    let table = oxikube_ports::StateTable::new(crate::cluster_tab::CLUSTER_TABS_TABLE).unwrap();
    let key = oxikube_ports::StateKey::new(MAIN_WINDOW_ID).unwrap();
    use oxikube_ports::StatePort as _;
    for garbage in [
        serde_json::json!("text"),
        serde_json::json!({ "open": "nope" }),
        serde_json::json!({ "version": 99, "open": [] }),
    ] {
        block_on(state.table_put(&table, &key, garbage.clone())).expect("put");
        assert_eq!(block_on(store.load()).expect("load"), None, "{garbage}");
    }
    let _ = cx;
}
