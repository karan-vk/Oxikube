//! The startup placeholder (E05-S13): the main window opens at once with the default layout,
//! marked "Restoring layout…", while the saved layout is read in the background; the saved layout
//! replaces it when the read completes, and a failed read leaves it usable.

use std::sync::Arc;

use futures::executor::block_on;
use gpui::{Entity, TestAppContext, VisualTestContext, WindowHandle};
use oxikube_domain::OxiError;
use oxikube_testkit::fakes::FakeStatePort;
use oxikube_ui::root::Root;

use super::{MainView, open_main_window_restoring};
use crate::actions::CloseActiveItem;
use crate::persistence::RestoreStatus;
use crate::persistence::tests::{gated::GatedState, open, store, window};
use crate::{Workspace, test_support::TestItem};

/// A fake holding the saved layout of a workspace with one item, `saved`. Also initialises the
/// component library, the workspace actions and the test item builder (through `window`).
fn saved_layout(cx: &mut TestAppContext) -> Arc<FakeStatePort> {
    let (ws, mut vcx) = window(cx);
    open(&ws, &mut vcx, "saved");
    let layout = vcx.update(|_, cx| ws.read(cx).serialize_layout(cx));
    let fake = Arc::new(FakeStatePort::new());
    block_on(store(fake.clone()).save(&layout)).expect("the fake stores it");
    fake
}

/// Opens the app's first window the way the binary does, over `state`.
fn open_restoring(
    cx: &mut TestAppContext,
    state: Arc<dyn oxikube_ports::StatePort>,
) -> (WindowHandle<Root>, VisualTestContext) {
    let handle = cx
        .update(|cx| open_main_window_restoring(cx, store(state), |content, _| content))
        .expect("the main window opens");
    let mut vcx = VisualTestContext::from_window(handle.into(), cx);
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    (handle, vcx)
}

fn main_view(vcx: &mut VisualTestContext) -> Entity<MainView> {
    vcx.update(|window, cx| {
        window
            .root::<Root>()
            .flatten()
            .expect("the Root")
            .read(cx)
            .view()
            .clone()
            .downcast::<MainView>()
            .expect("the main view")
    })
}

fn workspace(vcx: &mut VisualTestContext) -> Entity<Workspace> {
    let main = main_view(vcx);
    vcx.update(|_, cx| main.read(cx).workspace().clone())
}

fn marker_shown(vcx: &mut VisualTestContext) -> bool {
    vcx.debug_bounds("layout-restoring").is_some()
}

#[gpui::test]
fn the_placeholder_shows_until_the_saved_layout_replaces_it(cx: &mut TestAppContext) {
    let fake = saved_layout(cx);
    let (gated, open_gate) = GatedState::new(fake);
    let (_handle, mut vcx) = open_restoring(cx, gated);

    // First frame: drawn without waiting for the store, default layout, marked as restoring.
    let main = main_view(&mut vcx);
    assert!(vcx.update(|_, cx| main.read(cx).is_restoring(cx)));
    assert!(marker_shown(&mut vcx), "the title bar says it is restoring");
    assert!(vcx.debug_bounds("window-title").is_some());
    let ws = workspace(&mut vcx);
    assert!(
        vcx.update(|_, cx| ws.read(cx).is_blank()),
        "the default layout"
    );

    open_gate.send(()).expect("the restore is waiting");
    vcx.run_until_parked();

    assert!(!vcx.update(|_, cx| main.read(cx).is_restoring(cx)));
    assert!(!marker_shown(&mut vcx), "the marker goes away");
    vcx.update(|_, cx| {
        let persistence = main.read(cx).persistence().expect("persisted");
        assert!(matches!(
            persistence.read(cx).status(),
            RestoreStatus::Restored(report) if report.restored_items == 1
        ));
        assert_eq!(
            ws.read(cx).items().count(),
            1,
            "the saved layout replaced it"
        );
    });
    assert!(vcx.debug_bounds("item-saved").is_some(), "and is drawn");
}

#[gpui::test]
fn a_failed_restore_leaves_the_placeholder_usable(cx: &mut TestAppContext) {
    // Initialise the component library, actions and the test item builder.
    let _ = saved_layout(cx);
    let failing = Arc::new(FakeStatePort::new());
    failing
        .script()
        .table_get
        .push_err(OxiError::internal("disk on fire"));
    let (_handle, mut vcx) = open_restoring(cx, failing);

    let main = main_view(&mut vcx);
    vcx.update(|_, cx| {
        assert!(!main.read(cx).is_restoring(cx));
        let persistence = main.read(cx).persistence().expect("persisted");
        assert!(matches!(
            persistence.read(cx).status(),
            RestoreStatus::Failed(_)
        ));
    });
    assert!(!marker_shown(&mut vcx), "no restoring marker left behind");

    // The default layout works: items open, and workspace actions dispatch to it.
    let ws = workspace(&mut vcx);
    vcx.update(|window, cx| {
        let item = TestItem::build("after-failure".to_owned(), cx);
        ws.update(cx, |ws, cx| ws.open_item(item, window, cx));
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("item-after-failure").is_some());
    vcx.dispatch_action(CloseActiveItem);
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| ws.read(cx).is_blank()), "the action ran");
}

#[gpui::test]
fn a_plain_main_window_does_not_restore(cx: &mut TestAppContext) {
    let _ = saved_layout(cx);
    let handle = cx
        .update(super::open_main_window)
        .expect("the main window opens");
    let mut vcx = VisualTestContext::from_window(handle.into(), cx);
    vcx.run_until_parked();
    let main = main_view(&mut vcx);
    vcx.update(|_, cx| {
        assert!(main.read(cx).persistence().is_none());
        assert!(!main.read(cx).is_restoring(cx));
    });
    assert!(!marker_shown(&mut vcx));
}
