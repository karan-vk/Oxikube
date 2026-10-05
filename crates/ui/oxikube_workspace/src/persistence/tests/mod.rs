//! Tests of the layout store and the persistence controller, on `FakeStatePort`.

mod controller;
mod gated;
mod store;

use std::sync::Arc;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use oxikube_testkit::fakes::{FakeStatePort, StateCall};
use oxikube_ui::root::Root;
use serde_json::Value;

use super::{LAYOUT_TABLE, LayoutPersistence, LayoutStore, MAIN_WINDOW_ID, SerializedWorkspace};
use crate::{
    Workspace,
    test_support::{TestItem, register_test_item},
};

/// A window with a workspace, the test item builder registered.
pub(super) fn window(cx: &mut TestAppContext) -> (Entity<Workspace>, VisualTestContext) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        crate::actions::register(cx);
        register_test_item(cx);
    });
    let mut workspace = None;
    let window = cx.add_window(|window, cx| {
        let entity = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    (workspace.expect("the window was built"), vcx)
}

pub(super) fn open(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, title: &str) {
    let title = title.to_owned();
    vcx.update(|window, cx| {
        let item = TestItem::build(title, cx);
        ws.update(cx, |ws, cx| ws.open_item(item, window, cx));
    });
    vcx.run_until_parked();
}

pub(super) fn store(state: Arc<dyn oxikube_ports::StatePort>) -> LayoutStore {
    LayoutStore::new(state, MAIN_WINDOW_ID).expect("a valid window id")
}

/// Starts persistence on `ws` over `state` with a 500 ms debounce.
pub(super) fn start(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    state: Arc<dyn oxikube_ports::StatePort>,
) -> Entity<LayoutPersistence> {
    let persistence =
        vcx.update(|window, cx| LayoutPersistence::start(ws, store(state), window, cx));
    vcx.run_until_parked();
    persistence
}

/// The layouts written to the fake so far, oldest first.
pub(super) fn written(fake: &FakeStatePort) -> Vec<SerializedWorkspace> {
    fake.recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            StateCall::TablePut(table, _, row) if table.as_str() == LAYOUT_TABLE => {
                Some(SerializedWorkspace::from_json(row).expect("a layout"))
            }
            _ => None,
        })
        .collect()
}

/// The titles of the items in a saved centre, in order.
pub(super) fn saved_titles(layout: &SerializedWorkspace) -> Vec<String> {
    fn walk(state: &oxikube_ui::dock::PanelState, out: &mut Vec<String>) {
        if let Some(d) = super::item_descriptor(state)
            && let Value::String(title) = d.state
        {
            out.push(title);
        }
        for child in &state.children {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(&layout.dock_area.center, &mut out);
    out
}
