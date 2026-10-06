//! The saved filter (`resource_table.persist_filter`): off by default, per kind, restored when
//! the table opens, never written while the setting is off.

use std::sync::Arc;

use gpui::{Entity, TestAppContext};
use oxikube_ports::{StatePort as _, StatePortExt as _};
use oxikube_settings::{Settings as _, SettingsStore};
use oxikube_testkit::{FakeStatePort, pod};

use super::fixture::Fixture;
use crate::filter::{ResourceTableSettings, SavedFilter, filter_key};
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
    let saved = SavedFilter::new(f.state.clone(), &super::pods_kind().gvk).expect("key");
    futures::executor::block_on(saved.load()).expect("read")
}

#[gpui::test]
fn the_setting_defaults_to_off_and_nothing_is_written(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with(pods());
    let table = f.open_pods();
    type_filter(&mut f, &table, "web");
    assert_eq!(f.names(&table), ["web-1", "web-2"]);
    assert_eq!(saved(&f), None, "off: not saved");

    set_persist(&mut f, false);
    type_filter(&mut f, &table, "db");
    assert_eq!(saved(&f), None);
}

#[gpui::test]
fn with_the_setting_on_the_filter_is_saved_per_kind_and_restored(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    let mut f = Fixture::with_state(cx, state.clone());
    set_persist(&mut f, true);
    f.connect_with(pods());
    let table = f.open_pods();
    assert_eq!(f.names(&table).len(), 3, "nothing saved yet");
    type_filter(&mut f, &table, "web");
    f.settle();
    assert_eq!(saved(&f).as_deref(), Some("web"));
    assert!(
        futures::executor::block_on(state.kv_get(&filter_key(&super::pods_kind().gvk).unwrap()))
            .expect("read")
            .is_some()
    );
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
    let key = filter_key(&super::pods_kind().gvk).unwrap();
    futures::executor::block_on(state.kv_set_as(&key, &"not a filter row")).expect("write");
    let mut f = Fixture::with_state(cx, state);
    set_persist(&mut f, true);
    f.connect_with(pods());
    let table = f.open_pods();
    f.settle();
    assert_eq!(f.names(&table).len(), 3);
}
