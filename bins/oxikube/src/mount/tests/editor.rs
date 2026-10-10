//! The manifest editor in the running app (E10-S04): `editor::*` on the bus, `editor::NewManifest`
//! by its shipped key opening an editor in the shown cluster's tab, checked against the
//! connected cluster's schemas (the fake `SchemaPort` of the seeded cluster).

use std::sync::Arc;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;
use oxikube_editor::view::{ManifestEditor, VALIDATION_DEBOUNCE};
use oxikube_testkit::TestPorts;
use serde_json::json;

use super::App;
use crate::app_state::AppState;

fn run(app: &mut App, command: Command) -> bool {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let outcome = block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")));
    app.vcx.run_until_parked();
    outcome.is_ok()
}

#[gpui::test]
fn the_editor_commands_are_on_the_bus(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for id in [
        CommandId::EDITOR_NEW_MANIFEST,
        CommandId::EDITOR_TOGGLE_READ_ONLY,
        CommandId::EDITOR_TOGGLE_SOFT_WRAP,
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_editor"), "{id}");
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
}

#[gpui::test]
fn without_a_cluster_tab_a_new_manifest_opens_in_the_window(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(run(&mut app, Command::EditorNewManifest { cluster: None }));
    let editors = app.read(|ws, _| ws.items_of_type::<ManifestEditor>());
    assert_eq!(editors.len(), 1, "a tab of the window's workspace");
}

fn new_manifest_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-shift-e"
    } else {
        "ctrl-shift-e"
    }
}

#[gpui::test]
fn the_key_opens_an_editor_in_the_cluster_tab_checked_against_its_schemas(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let schema = JsonSchema::from_value(&json!({
        "type": "object",
        "properties": {
            "apiVersion": {"type": "string"},
            "kind": {"type": "string"},
            "data": {"type": "object", "additionalProperties": {"type": "string"}}
        }
    }));
    ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .schemas
        .insert(
            TestPorts::cluster_id(),
            Gvk::new("", "v1", "ConfigMap"),
            Arc::new(schema),
        );
    let mut app = App::start(cx, ports);
    app.press("enter");
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    let workspace = app.vcx.update(|_, cx| tab.read(cx).workspace().clone());

    app.press(new_manifest_key());
    let editor = app
        .vcx
        .update(|_, cx| workspace.read(cx).items_of_type::<ManifestEditor>())
        .pop()
        .expect("the editor opened in the cluster tab");
    let text = "apiVersion: v1\nkind: ConfigMap\nmetdata: {}\n";
    app.vcx.simulate_input(text);
    app.vcx.executor().advance_clock(VALIDATION_DEBOUNCE);
    app.vcx.run_until_parked();
    let problems = app.vcx.update(|_, cx| editor.read(cx).model().problems());
    assert_eq!(
        (problems.errors, problems.warnings),
        (0, 1),
        "`metdata` is an unknown field of the cluster's ConfigMap schema"
    );
}
