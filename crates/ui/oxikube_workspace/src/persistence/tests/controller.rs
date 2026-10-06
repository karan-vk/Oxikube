//! `LayoutPersistence`: restore at start, debounced and deduplicated saves, flush, failures.

use std::{sync::Arc, time::Duration};

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_domain::OxiError;
use oxikube_ports::{StateKey, StatePort, StateTable};
use oxikube_testkit::fakes::FakeStatePort;
use oxikube_ui::{Unscaled, dock::PanelInfo};
use serde_json::json;

use super::{gated::GatedState, open, saved_titles, start, store, window, written};
use crate::persistence::{
    LAYOUT_TABLE, LayoutPersistence, MAIN_WINDOW_ID, PersistenceEvent, RestoreStatus, SAVE_DEBOUNCE,
};
use crate::{item::ITEM_PANEL_NAME, panel::DockPosition, test_support::TestPanel};

const LONG_ENOUGH: Duration = Duration::from_millis(600);

fn tick(vcx: &mut gpui::VisualTestContext, by: Duration) {
    vcx.executor().advance_clock(by);
    vcx.run_until_parked();
}

/// A fake holding the layout of a workspace with items `titles`.
fn saved_fake(cx: &mut TestAppContext, titles: &[&str]) -> Arc<FakeStatePort> {
    let (ws, mut vcx) = window(cx);
    for title in titles {
        open(&ws, &mut vcx, title);
    }
    let layout = vcx.update(|_, cx| ws.read(cx).serialize_layout(cx));
    let fake = Arc::new(FakeStatePort::new());
    block_on(store(fake.clone()).save(&layout)).unwrap();
    fake.clear_calls();
    fake
}

#[gpui::test]
async fn the_saved_layout_is_restored_at_start(cx: &mut TestAppContext) {
    let fake = saved_fake(cx, &["a", "b"]);
    let (ws, mut vcx) = window(cx);
    let persistence = start(&ws, &mut vcx, fake.clone());
    vcx.update(|_, cx| {
        match persistence.read(cx).status() {
            RestoreStatus::Restored(report) => assert_eq!(report.restored_items, 2),
            other => panic!("{other:?}"),
        }
        assert_eq!(ws.read(cx).items().count(), 2);
    });
    assert!(written(&fake).is_empty(), "restoring writes nothing back");
    tick(&mut vcx, LONG_ENOUGH);
    assert!(
        written(&fake).is_empty(),
        "the restored layout is not rewritten"
    );
}

#[gpui::test]
async fn first_launch_has_nothing_saved_and_the_first_change_is_written(cx: &mut TestAppContext) {
    let fake = Arc::new(FakeStatePort::new());
    let (ws, mut vcx) = window(cx);
    let persistence = start(&ws, &mut vcx, fake.clone());
    vcx.update(|_, cx| assert_eq!(persistence.read(cx).status(), &RestoreStatus::NothingSaved));
    assert!(written(&fake).is_empty());

    open(&ws, &mut vcx, "a");
    tick(&mut vcx, LONG_ENOUGH);
    let writes = written(&fake);
    assert_eq!(writes.len(), 1);
    assert_eq!(saved_titles(&writes[0]), ["a"]);
    assert!(writes[0].window.is_some(), "the window place is saved too");
}

#[gpui::test]
async fn nothing_is_written_while_the_restore_is_in_flight(cx: &mut TestAppContext) {
    let fake = saved_fake(cx, &["saved"]);
    let (gated, open_gate) = GatedState::new(fake.clone());
    let (ws, mut vcx) = window(cx);
    let persistence = start(&ws, &mut vcx, gated);
    vcx.update(|_, cx| assert!(persistence.read(cx).is_restoring()));

    // The user acts before the layout arrives: the empty startup layout must not overwrite it.
    open(&ws, &mut vcx, "early");
    tick(&mut vcx, LONG_ENOUGH);
    assert!(written(&fake).is_empty());
    let stored = block_on(fake.table_get(
        &StateTable::new(LAYOUT_TABLE).unwrap(),
        &StateKey::new(MAIN_WINDOW_ID).unwrap(),
    ))
    .unwrap();
    assert!(stored.is_some(), "the saved layout is untouched");

    open_gate.send(()).unwrap();
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        // The user's item stays (restore never closes open items); the saved item was not forced in.
        assert!(matches!(
            persistence.read(cx).status(),
            RestoreStatus::Restored(r) if r.centre_kept
        ));
        assert_eq!(ws.read(cx).items().count(), 1);
    });
    // What changed during the restore is written once it is over.
    tick(&mut vcx, LONG_ENOUGH);
    let writes = written(&fake);
    assert_eq!(writes.len(), 1);
    assert_eq!(saved_titles(&writes[0]), ["early"]);
}

#[gpui::test]
async fn rapid_changes_are_coalesced_into_one_write(cx: &mut TestAppContext) {
    let fake = Arc::new(FakeStatePort::new());
    let (ws, mut vcx) = window(cx);
    start(&ws, &mut vcx, fake.clone());

    for title in ["a", "b", "c"] {
        open(&ws, &mut vcx, title);
        tick(&mut vcx, SAVE_DEBOUNCE / 4);
    }
    assert!(
        written(&fake).is_empty(),
        "still inside the debounce window"
    );
    tick(&mut vcx, LONG_ENOUGH);
    let writes = written(&fake);
    assert_eq!(writes.len(), 1, "one write for three changes");
    assert_eq!(saved_titles(&writes[0]), ["a", "b", "c"]);
}

#[gpui::test]
async fn an_unchanged_layout_is_not_written_again(cx: &mut TestAppContext) {
    let fake = Arc::new(FakeStatePort::new());
    let (ws, mut vcx) = window(cx);
    start(&ws, &mut vcx, fake.clone());
    open(&ws, &mut vcx, "a");
    tick(&mut vcx, LONG_ENOUGH);
    assert_eq!(written(&fake).len(), 1);

    // A layout event that changes nothing.
    vcx.update(|_, cx| ws.update(cx, |_, cx| cx.emit(crate::WorkspaceEvent::LayoutChanged)));
    tick(&mut vcx, LONG_ENOUGH);
    assert_eq!(written(&fake).len(), 1);
}

#[gpui::test]
async fn flush_writes_pending_changes_without_waiting(cx: &mut TestAppContext) {
    let fake = Arc::new(FakeStatePort::new());
    let (ws, mut vcx) = window(cx);
    let persistence = start(&ws, &mut vcx, fake.clone());
    open(&ws, &mut vcx, "a");
    assert!(written(&fake).is_empty());

    let flushed = vcx.update(|window, cx| persistence.update(cx, |p, cx| p.flush(window, cx)));
    vcx.executor().run_until_parked();
    block_on(flushed);
    assert_eq!(written(&fake).len(), 1);

    // The debounce timer that was pending is gone, and nothing is left to write.
    tick(&mut vcx, LONG_ENOUGH);
    assert_eq!(written(&fake).len(), 1);
    let again = vcx.update(|window, cx| persistence.update(cx, |p, cx| p.flush(window, cx)));
    block_on(again);
    assert_eq!(
        written(&fake).len(),
        1,
        "an unchanged layout is not flushed twice"
    );
}

#[gpui::test]
async fn quitting_flushes_the_pending_layout(cx: &mut TestAppContext) {
    let fake = Arc::new(FakeStatePort::new());
    let (ws, mut vcx) = window(cx);
    start(&ws, &mut vcx, fake.clone());
    open(&ws, &mut vcx, "a");
    assert!(written(&fake).is_empty(), "still debouncing");

    cx.update(|cx| cx.shutdown());
    cx.run_until_parked();
    let writes = written(&fake);
    assert_eq!(writes.len(), 1, "the quit hook wrote it");
    assert_eq!(saved_titles(&writes[0]), ["a"]);
}

#[gpui::test]
async fn a_failed_write_is_retried_on_the_next_change(cx: &mut TestAppContext) {
    let fake = Arc::new(FakeStatePort::new());
    let (ws, mut vcx) = window(cx);
    start(&ws, &mut vcx, fake.clone());
    fake.script()
        .table_put
        .push_err(OxiError::internal("disk full"));

    open(&ws, &mut vcx, "a");
    tick(&mut vcx, LONG_ENOUGH);
    assert_eq!(written(&fake).len(), 1, "attempted, and failed");
    open(&ws, &mut vcx, "b");
    tick(&mut vcx, LONG_ENOUGH);
    let writes = written(&fake);
    assert_eq!(writes.len(), 2);
    assert_eq!(saved_titles(&writes[1]), ["a", "b"]);
    block_on(async {
        let stored = fake
            .table_get(
                &StateTable::new(LAYOUT_TABLE).unwrap(),
                &StateKey::new(MAIN_WINDOW_ID).unwrap(),
            )
            .await
            .unwrap();
        assert!(stored.is_some(), "the retry landed");
    });
}

#[gpui::test]
async fn a_newer_layout_is_discarded_and_replaced_by_the_next_save(cx: &mut TestAppContext) {
    let fake = Arc::new(FakeStatePort::new());
    block_on(fake.table_put(
        &StateTable::new(LAYOUT_TABLE).unwrap(),
        &StateKey::new(MAIN_WINDOW_ID).unwrap(),
        json!({ "version": 99, "future": true }),
    ))
    .unwrap();
    fake.clear_calls();
    let (ws, mut vcx) = window(cx);
    let persistence = start(&ws, &mut vcx, fake.clone());
    vcx.update(|_, cx| {
        assert!(matches!(
            persistence.read(cx).status(),
            RestoreStatus::Discarded(_)
        ));
        assert!(ws.read(cx).is_blank(), "the default layout stays");
    });
    open(&ws, &mut vcx, "a");
    tick(&mut vcx, LONG_ENOUGH);
    assert_eq!(written(&fake).len(), 1);
}

#[gpui::test]
async fn a_failing_store_keeps_the_default_layout_and_saving_resumes(cx: &mut TestAppContext) {
    let fake = Arc::new(FakeStatePort::new());
    fake.script()
        .table_get
        .push_err(OxiError::internal("cannot read"));
    let (ws, mut vcx) = window(cx);
    let persistence = start(&ws, &mut vcx, fake.clone());
    vcx.update(|_, cx| {
        assert!(matches!(
            persistence.read(cx).status(),
            RestoreStatus::Failed(_)
        ));
    });
    open(&ws, &mut vcx, "a");
    tick(&mut vcx, LONG_ENOUGH);
    assert_eq!(written(&fake).len(), 1);
}

#[gpui::test]
async fn the_finish_of_a_restore_is_announced(cx: &mut TestAppContext) {
    let fake = Arc::new(FakeStatePort::new());
    let (gated, open_gate) = GatedState::new(fake);
    let (ws, mut vcx) = window(cx);
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let persistence: gpui::Entity<LayoutPersistence> =
        vcx.update(|window, cx| LayoutPersistence::start(&ws, store(gated), window, cx));
    {
        let events = events.clone();
        vcx.update(|_, cx| {
            cx.subscribe(&persistence, move |_, event: &PersistenceEvent, _| {
                events.lock().unwrap().push(event.clone());
            })
            .detach();
        });
    }
    vcx.run_until_parked();
    assert!(events.lock().unwrap().is_empty());
    open_gate.send(()).unwrap();
    vcx.run_until_parked();
    assert_eq!(
        *events.lock().unwrap(),
        [PersistenceEvent::RestoreFinished(
            RestoreStatus::NothingSaved
        )]
    );
}

/// Adds a left [`TestPanel`] to `ws`, so the workspace has a dock a saved layout can resize.
fn add_left_panel(ws: &gpui::Entity<crate::Workspace>, vcx: &mut gpui::VisualTestContext) {
    vcx.update(|window, cx| {
        let panel = TestPanel::build(DockPosition::Left, "nav", cx);
        ws.update(cx, |ws, cx| ws.add_panel(panel, window, cx));
    });
    vcx.run_until_parked();
}

#[gpui::test]
async fn a_layout_of_only_skipped_items_is_not_overwritten_by_the_restore(cx: &mut TestAppContext) {
    // Saved with a resized left dock and one item whose kind is no longer registered.
    let (ws, mut vcx) = window(cx);
    add_left_panel(&ws, &mut vcx);
    open(&ws, &mut vcx, "pods");
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.resize_dock(DockPosition::Left, Unscaled(333.), window, cx)
        })
    });
    let mut layout = vcx.update(|_, cx| ws.read(cx).serialize_layout(cx));
    fn rename(state: &mut oxikube_ui::dock::PanelState) {
        if state.panel_name == ITEM_PANEL_NAME {
            state.info = PanelInfo::panel(json!({ "kind": "gone::Pods", "state": null }));
        }
        state.children.iter_mut().for_each(rename);
    }
    rename(&mut layout.dock_area.center);
    let fake = Arc::new(FakeStatePort::new());
    block_on(store(fake.clone()).save(&layout)).unwrap();
    fake.clear_calls();

    let (fresh, mut vcx) = window(cx);
    add_left_panel(&fresh, &mut vcx);
    let persistence = start(&fresh, &mut vcx, fake.clone());
    vcx.update(|_, cx| {
        assert!(matches!(
            persistence.read(cx).status(),
            RestoreStatus::Restored(r) if !r.centre_restored && !r.centre_kept && r.skipped_items.len() == 1
        ));
    });
    // The dock was resized by the restore, which raised a layout event.
    tick(&mut vcx, LONG_ENOUGH);
    assert!(
        written(&fake).is_empty(),
        "the skipped item stays in the store until the user changes something"
    );
}
