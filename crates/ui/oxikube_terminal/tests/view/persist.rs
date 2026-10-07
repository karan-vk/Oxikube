//! Layout persistence of terminal tabs: what is stored is the backend descriptor (kind, program,
//! directory, cluster, namespace) and nothing else (no scrollback, no title the process set, no
//! environment); restoring starts a fresh process from it, docked where it was.

use std::sync::Arc;

use futures::executor::block_on;
use oxikube_ports::StatePort;
use oxikube_terminal::view::{
    BackendDescriptor, TERMINAL_ITEM_KIND, TerminalView, ensure_terminal_panel,
};
use oxikube_testkit::FakeStatePort;
use oxikube_workspace::persistence::{LayoutStore, LoadOutcome, SerializedWorkspace};
use oxikube_workspace::{DockPosition, ItemRegistry};

use super::*;

/// A cluster shell in `/work`, namespace `shop`, docked at the bottom.
fn cluster_shell() -> BackendDescriptor {
    BackendDescriptor::local(Some(cluster()))
        .in_namespace(Some("shop".into()))
        .with_shell("/bin/zsh", vec![])
        .in_dir("/work")
}

fn docked_terminal(h: &mut Harness, descriptor: BackendDescriptor) -> Entity<TerminalView> {
    let ws = h.ws.clone();
    let dispatcher: Rc<dyn CommandDispatcher> = h.recorder.clone();
    let view = h.terminal(descriptor);
    let opened = view.clone();
    h.vcx.update(|window, cx| {
        ensure_terminal_panel(&ws, Some(cluster()), Some(dispatcher), window, cx);
        ws.update(cx, |ws, cx| {
            ws.open_item_in_dock(Box::new(opened), DockPosition::Bottom, true, window, cx)
        })
    });
    h.frame();
    view
}

/// Saves `h`'s layout through a `LayoutStore` on a fake `StatePort` and reads it back.
fn save_and_load(h: &mut Harness) -> (SerializedWorkspace, String) {
    let ws = h.ws.clone();
    let layout = h.vcx.update(|_, cx| ws.read(cx).serialize_layout(cx));
    let state: Arc<dyn StatePort> = Arc::new(FakeStatePort::new());
    let store = LayoutStore::new(state, "main").expect("a store");
    block_on(store.save(&layout)).expect("saved");
    let loaded = match block_on(store.load()).expect("loaded") {
        LoadOutcome::Loaded(layout) => layout,
        other => panic!("the layout comes back: {other:?}"),
    };
    let json = serde_json::to_string(&loaded.to_json()).expect("json");
    (loaded, json)
}

#[gpui::test]
fn only_the_descriptor_is_stored_never_the_screen_title_or_environment(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let view = docked_terminal(&mut h, cluster_shell());
    h.backend(0).output(
        "export KUBECONFIG=/tmp/oxikube-1/kc.yaml\r\nTOKEN=s3cr3t-value\r\n\x1b]0;ssh prod-db\x07",
    );
    h.frame();
    assert_eq!(h.row(&view, 1), "TOKEN=s3cr3t-value", "on screen");

    let (_, json) = save_and_load(&mut h);
    assert!(json.contains(TERMINAL_ITEM_KIND), "{json}");
    let saved_state = h.vcx.update(|_, cx| {
        use oxikube_workspace::Item as _;
        view.read(cx).serialize(cx).expect("a state")
    });
    assert_eq!(
        saved_state,
        cluster_shell().to_state(),
        "the descriptor, whole"
    );
    for leaked in ["s3cr3t", "TOKEN", "KUBECONFIG", "kc.yaml", "prod-db"] {
        assert!(
            !json.contains(leaked),
            "{leaked} must not be stored: {json}"
        );
    }
}

#[gpui::test]
fn restoring_starts_a_fresh_process_from_the_descriptor_in_the_dock(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    docked_terminal(&mut h, cluster_shell());
    h.backend(0).output("old output\r\n");
    h.frame();
    let (layout, _) = save_and_load(&mut h);

    // A second window of the same app, set up the way a cluster tab is before its restore.
    let (fresh, mut vcx) = oxikube_workspace::test_support::open_workspace(cx);
    let dispatcher: Rc<dyn CommandDispatcher> = h.recorder.clone();
    vcx.update(|window, cx| {
        ensure_terminal_panel(&fresh, Some(cluster()), Some(dispatcher), window, cx);
    });
    let report =
        vcx.update(|window, cx| fresh.update(cx, |ws, cx| ws.restore_layout(&layout, window, cx)));
    vcx.run_until_parked();
    assert_eq!(report.restored_items, 1, "{report:?}");
    assert!(report.skipped_items.is_empty(), "{report:?}");

    assert_eq!(
        h.launches(),
        [cluster_shell(), cluster_shell()],
        "started again"
    );
    let restored = vcx.update(|_, cx| fresh.read(cx).items_of_type::<TerminalView>());
    assert_eq!(restored.len(), 1);
    let restored = restored[0].clone();
    let docked = vcx.update(|_, cx| fresh.read(cx).item_dock(restored.entity_id(), cx));
    assert_eq!(docked, Some(DockPosition::Bottom), "where it was");
    let first_row = vcx.update(|_, cx| {
        let state = restored.read(cx).terminal().expect("running").clone();
        state.read(cx).snapshot().row_text(0)
    });
    assert_eq!(first_row, "", "a fresh screen: nothing is replayed");
    assert_eq!(h.launcher.backends.borrow().len(), 2, "a new backend");
}

#[gpui::test]
fn a_saved_terminal_of_another_version_is_skipped(cx: &mut TestAppContext) {
    let h = harness(cx);
    let mut vcx = h.vcx;
    let built = vcx.update(|window, cx| {
        let state = serde_json::json!({ "v": 99, "backend": { "kind": "local" } });
        ItemRegistry::build(TERMINAL_ITEM_KIND, &state, window, cx).is_some()
    });
    assert!(!built);
}
