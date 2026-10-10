//! The palette over recents kept in the state store (`StateRecents` over `FakeStatePort`).

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_app::{RecentsStore, StateRecents};
use oxikube_domain::command::CommandId;
use oxikube_ports::{StateKey, StatePort};
use oxikube_testkit::fakes::FakeStatePort;
use serde_json::json;

use super::{Fixture, declared_index};
use crate::command_palette::PaletteRequest;

fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    futures::executor::block_on(fut)
}

fn state_with(ids: &[&str]) -> Arc<FakeStatePort> {
    let state = Arc::new(FakeStatePort::new());
    block_on(state.kv_set(
        &StateKey::new("recents.commands").unwrap(),
        json!({ "v": 1, "ids": ids }),
    ))
    .unwrap();
    state
}

fn fixture(cx: &mut TestAppContext, recents: Arc<StateRecents>) -> Fixture {
    Fixture::with_recents(cx, declared_index(), &["web"], recents)
}

#[gpui::test]
fn the_commands_run_before_the_restart_head_the_list(cx: &mut TestAppContext) {
    // The previous run left these, most recent first.
    let state = state_with(&["view::ZoomReset", "view::ZoomOut"]);
    let recents = Arc::new(StateRecents::new(state));
    block_on(recents.load());

    let mut f = fixture(cx, recents);
    f.open();
    assert_eq!(
        &f.listed()[..2],
        [CommandId::VIEW_ZOOM_RESET, CommandId::VIEW_ZOOM_OUT]
    );
}

#[gpui::test]
fn a_dispatched_command_is_written_to_the_state_store(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    let recents = Arc::new(StateRecents::new(state.clone()));
    let mut f = fixture(cx, recents.clone());
    f.open();
    f.type_text("zoom out");
    f.keys("enter");
    assert_eq!(recents.recent(), [CommandId::VIEW_ZOOM_OUT]);

    // The writer (or the quit) flushes; the store then holds the id and nothing else.
    block_on(recents.flush());
    let stored = block_on(state.kv_get(&StateKey::new("recents.commands").unwrap()))
        .unwrap()
        .expect("written");
    assert_eq!(stored, json!({ "v": 1, "ids": ["view::ZoomOut"] }));

    // The next run starts with it.
    let next = Arc::new(StateRecents::new(state));
    block_on(next.load());
    assert_eq!(next.recent(), [CommandId::VIEW_ZOOM_OUT]);
}

#[gpui::test]
fn opening_the_palette_never_waits_for_the_database(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    let recents = Arc::new(StateRecents::new(state.clone()));
    let mut f = fixture(cx, recents);
    f.open();
    f.type_text("zoom out");
    f.keys("enter");
    f.open();
    assert!(
        state.recorded_calls().is_empty(),
        "opening, matching and confirming only touch memory"
    );
}

#[gpui::test]
fn a_failing_state_store_leaves_the_palette_working(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    state
        .script()
        .kv_get
        .push_err(oxikube_domain::OxiError::internal("disk I/O error"));
    state
        .script()
        .kv_set
        .push_err(oxikube_domain::OxiError::internal("disk I/O error"));
    let recents = Arc::new(StateRecents::new(state));
    block_on(recents.load());
    let mut f = fixture(cx, recents.clone());
    f.open();
    f.type_text("zoom out");
    f.keys("enter");
    block_on(recents.flush());
    f.open();
    assert_eq!(f.listed()[0], CommandId::VIEW_ZOOM_OUT, "still in memory");
}

#[gpui::test]
fn clearing_recents_forgets_them_and_the_next_open_lists_by_category(cx: &mut TestAppContext) {
    let state = state_with(&["view::ZoomReset", "view::ZoomOut"]);
    let recents = Arc::new(StateRecents::new(state.clone()));
    block_on(recents.load());
    let mut f = fixture(cx, recents.clone());
    f.open();
    let with = f.listed();
    f.keys("escape");

    let host = f.host.clone();
    f.vcx.update(|window, cx| {
        host.apply(PaletteRequest::ClearRecents, window, cx);
    });
    f.settle();
    assert!(recents.recent().is_empty());
    assert!(
        f.toasts()
            .iter()
            .any(|t| t.contains("Recent commands cleared")),
        "{:?}",
        f.toasts()
    );

    f.open();
    let without = f.listed();
    assert_ne!(with[..2], without[..2]);
    assert!(!without[..2].contains(&CommandId::VIEW_ZOOM_RESET));

    // And the empty list is what the next run reads.
    block_on(recents.flush());
    let next = Arc::new(StateRecents::new(state));
    block_on(next.load());
    assert!(next.recent().is_empty());
}
