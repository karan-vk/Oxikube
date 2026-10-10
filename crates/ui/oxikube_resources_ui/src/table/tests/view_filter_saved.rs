//! The saved filter (`resource_table.persist_filter`, on by default): per cluster and kind,
//! restored when the table opens, removed when the filter is cleared, never written while the
//! setting is off.

use std::sync::Arc;

use gpui::{Entity, TestAppContext};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{StatePort as _, StatePortExt as _};
use oxikube_settings::{Settings as _, SettingsStore};
use oxikube_testkit::{FakeStatePort, pod};

use super::fixture::{Fixture, cluster};
use crate::filter::{ResourceTableContent, ResourceTableSettings, SavedFilter, filter_key};
use crate::table::ResourceTable;

fn set_persist(f: &mut Fixture, on: bool) {
    f.vcx.update(|_, cx| {
        if cx.try_global::<SettingsStore>().is_none() {
            cx.set_global(SettingsStore::empty());
        }
        ResourceTableSettings::register(cx);
        ResourceTableSettings::override_global(ResourceTableSettings { persist_filter: on }, cx);
    });
}

fn pods() -> Vec<oxikube_domain::Resource> {
    ["web-1", "web-2", "db-0"]
        .iter()
        .map(|n| pod().namespace("x").name(*n).build())
        .collect()
}

fn type_filter(f: &mut Fixture, table: &Entity<ResourceTable>, text: &str) {
    let text = text.to_owned();
    f.vcx.update(|window, cx| {
        table.update(cx, |t, cx| t.set_filter_text(&text, window, cx));
    });
    f.settle();
}

fn saved(f: &Fixture) -> Option<String> {
    saved_in(f, &cluster())
}

fn saved_in(f: &Fixture, cluster: &ClusterId) -> Option<String> {
    let saved = SavedFilter::new(f.state.clone(), cluster, &super::pods_kind().gvk).expect("key");
    futures::executor::block_on(saved.load()).expect("read")
}

fn key_exists(state: &FakeStatePort, cluster: &ClusterId) -> bool {
    let key = filter_key(cluster, &super::pods_kind().gvk).unwrap();
    futures::executor::block_on(state.kv_get(&key))
        .expect("read")
        .is_some()
}

#[gpui::test]
fn the_setting_defaults_to_on_and_the_filter_is_written(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    // The setting at its default (the content of an empty settings file).
    let default = ResourceTableSettings::from_content(ResourceTableContent::default());
    assert!(default.persist_filter, "persisted by default");
    f.vcx.update(|_, cx| {
        if cx.try_global::<SettingsStore>().is_none() {
            cx.set_global(SettingsStore::empty());
        }
        ResourceTableSettings::register(cx);
        ResourceTableSettings::override_global(default, cx);
    });
    f.connect_with(pods());
    let table = f.open_pods();
    type_filter(&mut f, &table, "web");
    assert_eq!(f.names(&table), ["web-1", "web-2"]);
    assert_eq!(saved(&f).as_deref(), Some("web"));
}

#[gpui::test]
fn with_the_setting_off_nothing_is_written(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    set_persist(&mut f, false);
    f.connect_with(pods());
    let table = f.open_pods();
    type_filter(&mut f, &table, "db");
    assert_eq!(f.names(&table), ["db-0"]);
    assert_eq!(saved(&f), None);
}

#[gpui::test]
fn the_filter_is_per_cluster_and_kind(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    let mut f = Fixture::with_state(cx, state);
    set_persist(&mut f, true);
    f.connect_with(pods());
    let table = f.open_pods();
    type_filter(&mut f, &table, "web");
    f.settle();
    let other = ClusterId::new("/home/me/.kube/config", &ContextName::new("prod"));
    assert_eq!(saved(&f).as_deref(), Some("web"));
    assert_eq!(saved_in(&f, &other), None, "another cluster has its own");
    let deployments = SavedFilter::new(
        f.state.clone(),
        &cluster(),
        &oxikube_domain::ids::Gvk::new("apps", "v1", "Deployment"),
    )
    .expect("key");
    assert_eq!(
        futures::executor::block_on(deployments.load()).expect("read"),
        None,
        "another kind has its own"
    );
}

#[gpui::test]
fn escape_removes_the_stored_filter_and_a_reopened_table_is_unfiltered(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    let mut f = Fixture::with_state(cx, state.clone());
    set_persist(&mut f, true);
    f.connect_with(pods());
    let table = f.open_pods();
    f.keys(&table, "/ w e b");
    f.settle();
    assert_eq!(saved(&f).as_deref(), Some("web"));
    assert!(key_exists(&state, &cluster()));

    // The bar still has the focus.
    f.vcx.simulate_keystrokes("escape");
    f.settle();
    assert_eq!(f.names(&table).len(), 3, "the filter is gone");
    assert!(
        !key_exists(&state, &cluster()),
        "clearing removes the stored value, it does not store an empty one"
    );
    drop(table);
    drop(f);

    let mut again = Fixture::with_state(cx, state);
    set_persist(&mut again, true);
    again.connect_with(pods());
    let table = again.open_pods();
    again.settle();
    assert_eq!(again.names(&table).len(), 3, "restarted: still unfiltered");
}

#[gpui::test]
fn escape_in_the_rows_clears_the_filter_and_the_stored_value(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    let mut f = Fixture::with_state(cx, state.clone());
    set_persist(&mut f, true);
    f.connect_with(pods());
    let table = f.open_pods();
    f.keys(&table, "/ w e b enter");
    f.settle();
    assert_eq!(f.names(&table).len(), 2, "filtered");
    assert_eq!(saved(&f).as_deref(), Some("web"));

    // Enter returned the focus to the rows: escape is the rows' escape now.
    f.vcx.simulate_keystrokes("escape");
    f.settle();
    assert_eq!(f.names(&table).len(), 3, "the filter is gone");
    assert!(
        !key_exists(&state, &cluster()),
        "and so is the stored value"
    );
}

#[gpui::test]
fn escape_in_the_rows_clears_a_selection_before_the_filter(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    set_persist(&mut f, true);
    f.connect_with(pods());
    let table = f.open_pods();
    f.keys(&table, "/ w e b enter");
    f.update(&table, |t, cx| t.select_all(cx));
    assert_eq!(f.selected(&table).len(), 2);

    f.vcx.simulate_keystrokes("escape");
    f.settle();
    assert!(
        f.selected(&table).is_empty(),
        "the first escape drops the selection"
    );
    assert_eq!(f.names(&table).len(), 2, "the filter stays");

    f.vcx.simulate_keystrokes("escape");
    f.settle();
    assert_eq!(f.names(&table).len(), 3, "the second one clears the filter");
}

#[gpui::test]
fn with_the_setting_on_the_filter_is_saved_and_restored_after_a_restart(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    let mut f = Fixture::with_state(cx, state.clone());
    set_persist(&mut f, true);
    f.connect_with(pods());
    let table = f.open_pods();
    assert_eq!(f.names(&table).len(), 3, "nothing saved yet");
    type_filter(&mut f, &table, "web");
    f.settle();
    assert_eq!(saved(&f).as_deref(), Some("web"));
    assert!(key_exists(&state, &cluster()));
    // The column layout and the filter are separate rows.
    type_filter(&mut f, &table, "-l app=x");
    f.settle();
    f.vcx
        .executor()
        .advance_clock(crate::filter::SELECTOR_DEBOUNCE);
    f.settle();
    assert_eq!(saved(&f).as_deref(), Some("-l app=x"));
    type_filter(&mut f, &table, "web");
    drop(table);
    drop(f);

    // A new window over the same state ("the app restarted"): the table opens filtered.
    let mut again = Fixture::with_state(cx, state);
    set_persist(&mut again, true);
    again.connect_with(pods());
    let table = again.open_pods();
    again.settle();
    assert_eq!(
        again.names(&table),
        ["web-1", "web-2"],
        "the saved filter was restored"
    );
    let bar = again.vcx.update(|_, cx| table.read(cx).filter().clone());
    assert_eq!(
        again.vcx.update(|_, cx| bar.read(cx).text().to_owned()),
        "web"
    );
}

#[gpui::test]
fn with_the_setting_off_a_saved_filter_is_not_restored(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    let mut f = Fixture::with_state(cx, state.clone());
    set_persist(&mut f, true);
    f.connect_with(pods());
    let table = f.open_pods();
    type_filter(&mut f, &table, "web");
    f.settle();
    drop(table);
    drop(f);

    let mut again = Fixture::with_state(cx, state);
    set_persist(&mut again, false);
    again.connect_with(pods());
    let table = again.open_pods();
    again.settle();
    assert_eq!(again.names(&table).len(), 3, "off: every row");
}

#[gpui::test]
fn an_unreadable_saved_row_is_ignored(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    let key = filter_key(&cluster(), &super::pods_kind().gvk).unwrap();
    futures::executor::block_on(state.kv_set_as(&key, &"not a filter row")).expect("write");
    let mut f = Fixture::with_state(cx, state);
    set_persist(&mut f, true);
    f.connect_with(pods());
    let table = f.open_pods();
    f.settle();
    assert_eq!(f.names(&table).len(), 3);
}
