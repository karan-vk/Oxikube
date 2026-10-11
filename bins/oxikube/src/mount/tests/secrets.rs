//! ConfigMaps and Secrets in the running app (E12-S11), on the real init path with fake ports: the
//! Config section of the sidebar reaches both tables; a Secret's detail drawer shows its panel
//! masked, `x` reveals it in memory through the real bus, `shift-e` opens it in an editor tab with
//! its values as text, the table's "New" opens the "Create secret" dialog; a ConfigMap's detail
//! lists its keys. The commands and the `k8s.get_secret` tool are on the bus and the registry.

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_editor::view::{CreateSecretDialog, ManifestEditor};
use oxikube_resources_ui::detail::{DetailDrawer, DetailView};
use oxikube_resources_ui::kinds::{ConfigMapPanel, SecretPanel};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::TestPorts;
use serde_json::json;

use super::App;
use crate::app_state::AppState;

fn kind(name: &str, plural: &str) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", name),
        preferred: true,
        plural: plural.into(),
        singular: name.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
        namespaced: true,
    }
}

fn secret() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "creds", "namespace": "default", "resourceVersion": "3"},
        "type": "Opaque",
        "data": {"password": "aHVudGVyMi1wbGFpbnRleHQ="}
    }))
    .unwrap()
}

fn config_map() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "app", "namespace": "default", "resourceVersion": "2"},
        "data": {"app.yaml": "name: shop\n"}
    }))
    .unwrap()
}

fn start(cx: &mut TestAppContext) -> App {
    let ports = TestPorts::seeded();
    let cluster = ports.connector.ports_for(&TestPorts::cluster_id());
    cluster
        .discovery
        .set_kinds([kind("Secret", "secrets"), kind("ConfigMap", "configmaps")]);
    cluster.resources.insert(secret());
    cluster.resources.insert(config_map());
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    app
}

fn kinds_of_tables(app: &mut App) -> Vec<String> {
    let ws = app.tab_workspace();
    app.vcx.update(|_, cx| {
        ws.read(cx)
            .items_of_type::<ResourceTable>()
            .into_iter()
            .map(|table| table.read(cx).gvk().kind.to_string())
            .collect()
    })
}

fn drawer(app: &mut App) -> gpui::Entity<DetailView> {
    let ws = app.tab_workspace();
    app.vcx
        .update(|_, cx| {
            ws.read(cx)
                .panel::<DetailDrawer>()
                .and_then(|drawer| drawer.read(cx).view().cloned())
        })
        .expect("the detail drawer is open")
}

#[gpui::test]
fn the_commands_and_the_tool_are_in_the_app(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, owner, tool) in [
        (
            CommandId::SECRET_REVEAL,
            "oxikube_resources_ui",
            "k8s.secret_reveal",
        ),
        (
            CommandId::SECRET_EDIT_DECODED,
            "oxikube_editor",
            "k8s.secret_edit_decoded",
        ),
        (
            CommandId::SECRET_CREATE,
            "oxikube_editor",
            "k8s.secret_create",
        ),
    ] {
        assert_eq!(bus.owner(id), Some(owner), "{id}");
        assert_eq!(id.tool_name(), tool);
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
    let hooks = state
        .agent_hooks()
        .cloned()
        .expect("the mount installed the agent hooks");
    let names: Vec<String> = hooks
        .tools
        .defs()
        .into_iter()
        .map(|def| def.name.to_string())
        .collect();
    assert!(names.contains(&"k8s.get_secret".to_owned()), "{names:?}");
}

#[gpui::test]
fn the_config_section_reaches_the_secrets_table_and_x_reveals_a_secret(cx: &mut TestAppContext) {
    let mut app = start(cx);
    app.click("sidebar-entry-config/secrets");
    app.tick();
    assert_eq!(kinds_of_tables(&mut app), ["Secret"]);

    // Enter opens the detail drawer with the Secret panel: the keys, masked.
    app.click("td-0-0");
    app.press("enter");
    app.tick();
    assert!(
        app.drawn("secret-panel"),
        "the Secret panel is in the drawer"
    );
    assert!(app.drawn("secret-key-0"));
    assert!(!app.drawn("detail-secret-key-0"), "one list of keys");
    let view = drawer(&mut app);
    assert!(app.vcx.update(|_, cx| !view.read(cx).reveal_requested()));

    // `x` (DetailDrawer && kind == Secret) runs `secret::Reveal` on the real bus.
    app.press("x");
    app.tick();
    let shown = app.vcx.update(|_, cx| {
        view.read(cx)
            .revealed_value("password")
            .and_then(|v| v.text().map(str::to_owned))
    });
    assert_eq!(shown.as_deref(), Some("hunter2-plaintext"));
    // And again: hidden.
    app.press("x");
    app.tick();
    assert!(app.vcx.update(|_, cx| !view.read(cx).secret_values_shown()));
    let panel = app
        .vcx
        .update(|_, cx| view.read(cx).kind_panel().cloned())
        .and_then(|panel| panel.downcast::<SecretPanel>().ok());
    assert!(panel.is_some(), "the drawer's panel is the Secret panel");
}

#[gpui::test]
fn shift_e_opens_the_secret_in_an_editor_with_its_values_as_text(cx: &mut TestAppContext) {
    let mut app = start(cx);
    app.click("sidebar-entry-config/secrets");
    app.tick();
    app.click("td-0-0");
    // The table's key (`ResourceTable && kind == Secret`) acts on the cursor row.
    app.press("shift-e");
    app.tick();
    let ws = app.tab_workspace();
    let editors = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ManifestEditor>());
    assert_eq!(editors.len(), 1, "an editor tab opened");
    let editor = &editors[0];
    let (title, decoded) = app.vcx.update(|_, cx| {
        let editor = editor.read(cx);
        (
            editor.binding().map(|b| b.title().to_string()),
            editor.binding().is_some_and(|b| b.is_decoded()),
        )
    });
    assert_eq!(title.as_deref(), Some("Secret/creds"));
    assert!(decoded);
    let code = app.vcx.update(|_, cx| editor.read(cx).editor().clone());
    let text = app.vcx.update(|window, cx| {
        use oxikube_ui::editor::EditorApi as _;
        code.update(cx, |code, cx| code.api(window, cx).text().to_string())
    });
    assert!(text.contains("password: hunter2-plaintext"), "{text}");
}

#[gpui::test]
fn new_on_the_secrets_table_opens_the_create_dialog(cx: &mut TestAppContext) {
    let mut app = start(cx);
    app.click("sidebar-entry-config/secrets");
    app.tick();
    app.click("resource-table-create");
    app.tick();
    let ws = app.tab_workspace();
    let dialog = app.vcx.update(|_, cx| {
        ws.read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<CreateSecretDialog>()
    });
    assert!(dialog.is_some(), "the Create secret dialog is open");
    assert!(app.drawn("create-secret-dialog"));
}

#[gpui::test]
fn a_config_maps_detail_lists_its_keys_with_a_preview(cx: &mut TestAppContext) {
    let mut app = start(cx);
    app.click("sidebar-entry-config/configmaps");
    app.tick();
    assert_eq!(kinds_of_tables(&mut app), ["ConfigMap"]);
    app.click("td-0-0");
    app.press("enter");
    app.tick();
    app.tick();
    assert!(app.drawn("configmap-panel"));
    assert!(app.drawn("configmap-key-0"));
    let view = drawer(&mut app);
    let panel = app
        .vcx
        .update(|_, cx| view.read(cx).kind_panel().cloned())
        .and_then(|panel| panel.downcast::<ConfigMapPanel>().ok())
        .expect("the ConfigMap panel");
    app.click("configmap-key-0");
    assert_eq!(
        app.vcx
            .update(|_, cx| panel.read(cx).expanded().map(str::to_owned))
            .as_deref(),
        Some("app.yaml")
    );
    assert!(app.drawn("configmap-preview"));
}

#[gpui::test]
fn the_palette_offers_the_secret_commands_where_they_apply(cx: &mut TestAppContext) {
    use oxikube_app::command_bus::{CommandContext, Selection};
    use oxikube_domain::Capabilities;
    use oxikube_domain::command::ViewContext;

    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let offered = |ctx: &CommandContext| -> Vec<CommandId> {
        bus.list(ctx)
            .iter()
            .map(|info| info.id())
            .filter(|id| id.namespace() == "secret")
            .collect()
    };
    let session = |view, read_only| {
        let mut ctx = CommandContext::new(view);
        ctx.cluster_active = true;
        ctx.capabilities = Capabilities::all();
        ctx.read_only = read_only;
        ctx
    };
    let secret = Gvk::new("", "v1", "Secret");
    // A Secret selected in a table: all three, on a read-only cluster too (they open views; the
    // write is `resource::Apply`, which the guard refuses there).
    for read_only in [false, true] {
        let ctx = session(ViewContext::Table, read_only).selecting(Selection::one(secret.clone()));
        let ids = offered(&ctx);
        for id in [
            CommandId::SECRET_REVEAL,
            CommandId::SECRET_EDIT_DECODED,
            CommandId::SECRET_CREATE,
        ] {
            assert!(ids.contains(&id), "{id} (read-only {read_only}): {ids:?}");
        }
    }
    // Another kind selected: only the creator.
    let ctx =
        session(ViewContext::Table, false).selecting(Selection::one(Gvk::new("", "v1", "Pod")));
    assert_eq!(offered(&ctx), [CommandId::SECRET_CREATE]);
    // No cluster: none.
    assert_eq!(offered(&CommandContext::new(ViewContext::Table)), []);
}
