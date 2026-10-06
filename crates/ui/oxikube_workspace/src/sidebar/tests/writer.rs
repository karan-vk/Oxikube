//! The writer that persists the open and closed choices: ordered, and not clobbering what was
//! saved before the panel's own load returned.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_testkit::FakeStatePort;

use super::id;
use crate::sidebar::SidebarStore;
use crate::sidebar::writer::SidebarWriter;

fn choices(items: &[(&str, bool)]) -> BTreeMap<String, bool> {
    items.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
}

fn store() -> SidebarStore {
    SidebarStore::new(Arc::new(FakeStatePort::new()), &id("prod")).expect("a valid key")
}

fn saved(store: &SidebarStore) -> BTreeMap<String, bool> {
    block_on(store.load())
        .expect("readable")
        .map(|s| s.open)
        .unwrap_or_default()
}

#[gpui::test]
fn a_toggle_before_the_saved_state_is_read_keeps_the_earlier_choices(cx: &mut TestAppContext) {
    let store = store();
    block_on(store.save(&choices(&[("network", false), ("storage", false)]))).unwrap();

    // The first toggle of the session arrives before anything was read: its snapshot holds only
    // this session's choices.
    let writer = cx.update(|cx| SidebarWriter::spawn(store.clone(), cx));
    writer.save(choices(&[("workloads", false)]));
    cx.run_until_parked();

    assert_eq!(
        saved(&store),
        choices(&[("network", false), ("storage", false), ("workloads", false)]),
        "the row is the saved choices with the new one on top, not a replacement"
    );
}

#[gpui::test]
fn the_session_wins_over_what_was_saved(cx: &mut TestAppContext) {
    let store = store();
    block_on(store.save(&choices(&[("workloads", false)]))).unwrap();
    let writer = cx.update(|cx| SidebarWriter::spawn(store.clone(), cx));
    writer.save(choices(&[("workloads", true)]));
    cx.run_until_parked();
    assert_eq!(saved(&store), choices(&[("workloads", true)]));
}

#[gpui::test]
fn a_burst_of_toggles_ends_with_the_newest_snapshot(cx: &mut TestAppContext) {
    let store = store();
    let writer = cx.update(|cx| SidebarWriter::spawn(store.clone(), cx));
    let mut open = BTreeMap::new();
    for id in ["workloads", "config", "network", "storage"] {
        open.insert(id.to_owned(), false);
        writer.save(open.clone());
    }
    cx.run_until_parked();
    assert_eq!(saved(&store), open, "the last snapshot is the one on disk");

    // A write queued right before the panel is dropped still lands.
    open.insert("events".to_owned(), false);
    writer.save(open.clone());
    drop(writer);
    cx.run_until_parked();
    assert_eq!(saved(&store), open);
}
