//! Roles and ClusterRoles in the running app (E17-S05), on the real init path with fake ports:
//! Access Control in the sidebar reaches both tables, a row's detail shows the rule table, the
//! table's "New" and the detail's "Edit rules…" open the rule dialog, and saving it goes through
//! the real bus: the guard asks for a simple confirmation (risk medium), the confirmed command
//! reaches the cluster after a server dry run, a read-only cluster refuses it with a toast. The
//! commands and their tool stubs are on the bus.

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::command::CommandId;
use oxikube_rbac_ui::role::{RoleDialog, RolePanel, RuleDraft};
use oxikube_resources_ui::detail::{DetailDrawer, DetailView};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::kinds::core_kinds;
use oxikube_testkit::{ResourceCall, TestPorts};
use oxikube_workspace::modal::DialogModal;
use oxikube_workspace::sidebar::SidebarPanel;
use serde_json::json;

use super::App;
use crate::app_state::AppState;

fn cluster_role() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
        "metadata": {"name": "viewer", "resourceVersion": "4", "uid": "u-viewer"},
        "rules": [
            {"apiGroups": [""], "resources": ["pods", "pods/log"], "verbs": ["get", "list"]},
            {"nonResourceURLs": ["/healthz"], "verbs": ["get"]},
        ],
    }))
    .unwrap()
}

fn start(cx: &mut TestAppContext) -> App {
    let ports = TestPorts::seeded();
    let cluster = ports.connector.ports_for(&TestPorts::cluster_id());
    cluster.discovery.set_kinds(core_kinds());
    cluster.resources.insert(cluster_role());
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    app
}

fn dialog(app: &mut App) -> Option<gpui::Entity<RoleDialog>> {
    let ws = app.tab_workspace();
    app.vcx.update(|_, cx| {
        ws.read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<RoleDialog>()
    })
}

/// Opens the table behind the sidebar entry `id`, as a click on it does (Access Control is below
/// the fold of a test window).
fn open_entry(app: &mut App, id: &str) {
    let ws = app.tab_workspace();
    let panel = app
        .vcx
        .update(|_, cx| ws.read(cx).panel::<SidebarPanel>().expect("the sidebar"));
    app.vcx
        .update(|_, cx| panel.update(cx, |panel, cx| panel.activate(id, cx)));
    app.tick();
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
fn the_commands_and_their_tool_stubs_are_on_the_bus(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, owner, tool) in [
        (
            CommandId::ROLE_OPEN_DIALOG,
            "oxikube_rbac_ui::role",
            "k8s.role_open_dialog",
        ),
        (
            CommandId::ROLE_CREATE,
            "oxikube_app::role_ops",
            "k8s.role_create",
        ),
        (
            CommandId::ROLE_EDIT,
            "oxikube_app::role_ops",
            "k8s.role_edit",
        ),
        (
            CommandId::ROLE_DELETE,
            "oxikube_app::role_ops",
            "k8s.role_delete",
        ),
    ] {
        assert_eq!(bus.owner(id), Some(owner), "{id}");
        assert_eq!(id.tool_name(), tool);
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
}

#[gpui::test]
fn access_control_reaches_the_cluster_roles_table_and_its_detail_shows_the_rules(
    cx: &mut TestAppContext,
) {
    let mut app = start(cx);
    open_entry(&mut app, "access-control/clusterroles");
    assert_eq!(app.tables(), ["ClusterRole"]);
    app.click("td-0-0");
    app.press("enter");
    app.tick();
    app.tick();
    assert!(app.drawn("role-panel"), "the Role panel is in the drawer");
    assert!(app.drawn("role-rule-0"));
    assert!(app.drawn("role-rule-1"));
    let view = drawer(&mut app);
    let panel = app
        .vcx
        .update(|_, cx| view.read(cx).kind_panel().cloned())
        .and_then(|panel| panel.downcast::<RolePanel>().ok())
        .expect("the drawer's panel is the Role panel");
    let rows = app.vcx.update(|_, cx| panel.read(cx).rows().to_vec());
    assert_eq!(&*rows[0].resources, "pods, pods/log");
    assert_eq!(&*rows[1].non_resource_urls, "/healthz");
}

#[gpui::test]
fn new_on_the_roles_table_opens_the_rule_dialog(cx: &mut TestAppContext) {
    let mut app = start(cx);
    open_entry(&mut app, "access-control/roles");
    assert_eq!(app.tables(), ["Role"]);
    app.click("resource-table-create");
    app.tick();
    assert!(dialog(&mut app).is_some(), "the rule dialog is open");
    assert!(app.drawn("role-dialog"));
    assert!(app.drawn("role-namespace"), "a Role has a namespace");
}

#[gpui::test]
fn the_cluster_roles_table_lists_rules_and_aggregation_and_its_new_has_no_namespace(
    cx: &mut TestAppContext,
) {
    let mut app2 = start(cx);
    open_entry(&mut app2, "access-control/clusterroles");
    let ws = app2.tab_workspace();
    let table = app2
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
    let columns = app2.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .visible_ids()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
    });
    assert!(columns.contains(&"rules".to_owned()), "{columns:?}");
    assert!(columns.contains(&"aggregated".to_owned()), "{columns:?}");
    app2.click("resource-table-create");
    app2.tick();
    assert!(dialog(&mut app2).is_some());
    assert!(!app2.drawn("role-namespace"), "a ClusterRole has none");
}

#[gpui::test]
fn edit_rules_in_the_detail_opens_the_dialog_on_the_role_and_saving_asks_to_confirm(
    cx: &mut TestAppContext,
) {
    let mut app = start(cx);
    open_entry(&mut app, "access-control/clusterroles");
    app.click("td-0-0");
    app.press("enter");
    app.tick();
    app.tick();
    app.click("role-edit");
    app.tick();
    let dialog = dialog(&mut app).expect("Edit rules… opened the dialog");
    assert!(app.vcx.update(|_, cx| dialog.read(cx).is_edit()));
    let draft = app.vcx.update(|_, cx| dialog.read(cx).draft(cx));
    assert_eq!(draft.name, "viewer");
    assert_eq!(draft.rules.len(), 2);

    // Add a rule and save: nothing is written until the guard's confirmation is answered.
    app.vcx.update(|window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.add_rule(
                &RuleDraft {
                    api_groups: vec!["apps".into()],
                    resources: vec!["deployments".into()],
                    verbs: vec!["get".into()],
                    ..RuleDraft::default()
                },
                window,
                cx,
            );
        });
    });
    app.click("role-save");
    app.tick();
    let resources = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    assert!(resources.mutating_calls().is_empty(), "asked, not written");
    let ws = app.tab_workspace();
    let (title, message) = app.vcx.update(|_, cx| {
        let modal = ws
            .read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<DialogModal>()
            .expect("the guard's confirmation");
        let modal = modal.read(cx);
        (
            modal.title().to_string(),
            modal
                .message_text()
                .map(ToString::to_string)
                .unwrap_or_default(),
        )
    });
    assert_eq!(title, "Save the role?");
    assert!(
        message.contains("ClusterRole viewer") && message.contains("3 rules"),
        "{message}"
    );

    // Confirmed: the server's dry run, then the replace at the version the dialog read.
    app.press("enter");
    app.tick();
    app.tick();
    let writes = resources.mutating_calls();
    let replaces: Vec<(bool, String)> = writes
        .iter()
        .filter_map(|call| match call {
            ResourceCall::Replace {
                object, options, ..
            } => Some((
                options.dry_run,
                object["metadata"]["resourceVersion"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        replaces,
        [(true, "4".to_owned()), (false, "4".to_owned())],
        "{writes:?}"
    );
    let audit = app.ports.state.audit_log();
    let record = audit.last().expect("an audit record");
    assert_eq!(&*record.cmd, "role::Edit");
    assert_eq!(record.detail.as_deref(), Some("rules=3"));
}

#[gpui::test]
fn a_read_only_cluster_refuses_the_save_and_says_so(cx: &mut TestAppContext) {
    let mut app = start(cx);
    open_entry(&mut app, "access-control/roles");
    app.click("resource-table-create");
    app.tick();
    let dialog = dialog(&mut app).expect("the dialog");
    app.vcx.update(|window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.fill_name("reader", window, cx);
            dialog.fill_rule(
                0,
                &RuleDraft {
                    api_groups: vec![String::new()],
                    resources: vec!["pods".into()],
                    verbs: vec!["get".into()],
                    ..RuleDraft::default()
                },
                window,
                cx,
            );
        });
    });
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("the session is open");
    app.click("role-save");
    app.tick();
    assert!(
        app.toasts()
            .iter()
            .any(|toast| toast.contains("is read-only")),
        "the refusal is shown: {:?}",
        app.toasts()
    );
    let resources = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    assert!(resources.mutating_calls().is_empty());
}
