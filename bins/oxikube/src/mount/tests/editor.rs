//! The manifest editor in the running app (E10-S04): `editor::*` on the bus, `editor::NewManifest`
//! by its shipped key opening an editor in the shown cluster's tab, checked against the
//! connected cluster's schemas (the fake `SchemaPort` of the seeded cluster).
//!
//! E10-S06: `e` on a pods table row, the detail drawer's "Edit" and "View YAML in Editor" buttons
//! and the bus open a tab titled `kind/name` in the cluster's tab, reading the object fresh from
//! the connection's reader; the same object reuses its tab.

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
        CommandId::EDITOR_CONVERT_TO_JSON,
        CommandId::EDITOR_CONVERT_TO_YAML,
        CommandId::EDITOR_USE_JSON,
        CommandId::EDITOR_USE_YAML,
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

#[gpui::test]
fn the_open_commands_are_on_the_bus_and_are_row_actions(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, tool) in [
        (CommandId::RESOURCE_EDIT, "k8s.resource_edit"),
        (
            CommandId::RESOURCE_VIEW_YAML_IN_EDITOR,
            "k8s.resource_view_yaml_in_editor",
        ),
        (
            CommandId::EDITOR_TOGGLE_MANAGED_FIELDS,
            "app.editor_toggle_managed_fields",
        ),
        (
            CommandId::EDITOR_TOGGLE_SECRET_MASK,
            "app.editor_toggle_secret_mask",
        ),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_editor"), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
    let offered: Vec<_> = crate::mount::logs::row_actions()
        .specs()
        .iter()
        .map(|spec| spec.command)
        .collect();
    assert!(offered.contains(&CommandId::RESOURCE_EDIT));
    assert!(offered.contains(&CommandId::RESOURCE_VIEW_YAML_IN_EDITOR));
}

fn tab_editors(app: &mut App) -> Vec<gpui::Entity<ManifestEditor>> {
    let ws = app.tab_workspace();
    app.vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ManifestEditor>())
}

fn editor_title(app: &mut App, editor: &gpui::Entity<ManifestEditor>) -> String {
    app.vcx.update(|_, cx| {
        oxikube_workspace::Item::tab_content(editor.read(cx), cx)
            .title
            .to_string()
    })
}

fn editor_read_only(app: &mut App, editor: &gpui::Entity<ManifestEditor>) -> bool {
    app.vcx
        .update(|_, cx| editor.read(cx).editor().read(cx).is_read_only())
}

#[gpui::test]
fn e_on_a_pods_row_opens_the_pod_in_an_editable_tab_of_the_cluster(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_pods_table();
    app.press("e");
    app.tick();
    let editors = tab_editors(&mut app);
    assert_eq!(editors.len(), 1, "an editor in the cluster's tab");
    assert_eq!(editor_title(&mut app, &editors[0]), "Pod/web-running");
    assert!(!editor_read_only(&mut app, &editors[0]));
    let text = app.vcx.update(|window, cx| {
        let code = editors[0].read(cx).editor().clone();
        code.update(cx, |code, cx| {
            oxikube_ui::editor::EditorApi::text(&code.api(window, cx)).to_string()
        })
    });
    assert!(text.starts_with("apiVersion: v1\nkind: Pod\n"), "{text}");
    assert!(!text.contains("managedFields"));

    // The second `e` focuses the same tab.
    app.press("e");
    app.tick();
    assert_eq!(tab_editors(&mut app).len(), 1, "no duplicate tab");
}

#[gpui::test]
fn the_detail_drawer_opens_the_editor_from_its_header(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_pods_table();
    app.press("enter");
    app.tick();
    app.click("detail-view-editor");
    app.tick();
    let editors = tab_editors(&mut app);
    assert_eq!(editors.len(), 1);
    assert_eq!(editor_title(&mut app, &editors[0]), "Pod/web-running");
    assert!(
        editor_read_only(&mut app, &editors[0]),
        "View YAML in Editor opens read-only"
    );

    // "Edit" on the same object makes that tab editable instead of opening a second one.
    app.click("detail-edit");
    app.tick();
    let editors = tab_editors(&mut app);
    assert_eq!(editors.len(), 1);
    assert!(!editor_read_only(&mut app, &editors[0]));
}

fn convert_key(to: &str) -> String {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    format!("{modifier}-alt-{}", if to == "json" { "j" } else { "y" })
}

fn buffer(app: &mut App, editor: &gpui::Entity<ManifestEditor>) -> String {
    let code = app.vcx.update(|_, cx| editor.read(cx).editor().clone());
    app.vcx.update(|window, cx| {
        code.update(cx, |code, cx| {
            use oxikube_ui::editor::EditorApi as _;
            code.api(window, cx).text().to_string()
        })
    })
}

/// JSON mode in the running app (E10-S10): the shipped keys and the bus convert the focused
/// editor's buffer, and one undo brings the YAML back.
#[gpui::test]
fn the_convert_keys_and_commands_turn_the_buffer_into_json_and_back(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press(new_manifest_key());
    let editor = app
        .read(|ws, _| ws.items_of_type::<ManifestEditor>())
        .pop()
        .expect("the editor opened");
    app.vcx
        .simulate_input("# note\nkind: Pod\nmetadata:\n  name: web\n");
    app.vcx.run_until_parked();
    let yaml = buffer(&mut app, &editor);
    assert_eq!(yaml, "# note\nkind: Pod\nmetadata:\n  name: web\n");

    app.press(&convert_key("json"));
    app.vcx.run_until_parked();
    assert_eq!(
        buffer(&mut app, &editor),
        "{\n  \"kind\": \"Pod\",\n  \"metadata\": {\n    \"name\": \"web\"\n  }\n}\n"
    );
    let notice = app
        .vcx
        .update(|_, cx| editor.read(cx).notice().map(|n| n.text.to_string()));
    assert_eq!(
        notice.as_deref(),
        Some("Converted to JSON, but 1 comment dropped. Undo brings the YAML back.")
    );

    // The same through the bus (what the palette and agents send): back to YAML.
    assert!(run(&mut app, Command::EditorConvertToYaml));
    assert_eq!(
        buffer(&mut app, &editor),
        "kind: Pod\nmetadata:\n  name: web\n"
    );
    // Undo the conversion to YAML, then the conversion to JSON: the typed text is back.
    for _ in 0..2 {
        app.press(if cfg!(target_os = "macos") {
            "cmd-z"
        } else {
            "ctrl-z"
        });
        app.vcx.run_until_parked();
    }
    assert_eq!(buffer(&mut app, &editor), yaml);
}

/// The convert and mode commands are offered where a manifest editor has the focus and nowhere
/// else (the palette lists `bus.list(ctx)`), on a read-only cluster too: they edit text only.
#[gpui::test]
fn the_conversion_commands_are_offered_only_while_an_editor_has_the_focus(cx: &mut TestAppContext) {
    use oxikube_app::command_bus::CommandContext;
    use oxikube_domain::command::ViewContext;

    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let ids = [
        CommandId::EDITOR_CONVERT_TO_JSON,
        CommandId::EDITOR_CONVERT_TO_YAML,
        CommandId::EDITOR_USE_JSON,
        CommandId::EDITOR_USE_YAML,
    ];
    let offered = |ctx: &CommandContext| -> Vec<CommandId> {
        bus.list(ctx)
            .iter()
            .map(|info| info.id())
            .filter(|id| ids.contains(id))
            .collect()
    };
    for view in [
        ViewContext::Workspace,
        ViewContext::Table,
        ViewContext::Detail,
        ViewContext::Logs,
        ViewContext::Terminal,
        ViewContext::Palette,
    ] {
        assert_eq!(offered(&CommandContext::new(view)), [], "{view}");
    }
    let in_editor = offered(&CommandContext::new(ViewContext::Editor));
    assert_eq!(in_editor.len(), 4, "{in_editor:?}");
    let read_only = offered(&CommandContext::new(ViewContext::Editor).read_only(true));
    assert_eq!(read_only.len(), 4, "text edits are not cluster mutations");
}
