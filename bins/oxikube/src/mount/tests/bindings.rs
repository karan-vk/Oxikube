//! RoleBindings and ClusterRoleBindings in the running app (E17-S06), on the real init path with
//! fake ports: Access Control in the sidebar reaches both tables (Role ref and Subjects columns),
//! a row's detail shows the subjects and the role's rules, the table's "New" and the detail's
//! "Edit…" open the binding dialog, and saving goes through the real bus: the guard asks for a
//! simple confirmation (risk medium; cluster-admin called out), the confirmed command reaches the
//! cluster after a server dry run, a read-only cluster refuses it with a toast. The commands and
//! their tool stubs are on the bus.

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::command::CommandId;
use oxikube_rbac_ui::binding::{BindingDialog, BindingPanel, SubjectDraft, SubjectKind};
use oxikube_resources_ui::detail::{DetailDrawer, DetailView};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::kinds::core_kinds;
use oxikube_testkit::{ResourceCall, TestPorts};
use oxikube_workspace::modal::ConfirmDialog;
use oxikube_workspace::sidebar::SidebarPanel;
use serde_json::json;

use super::App;
use crate::app_state::AppState;

fn cluster_role_binding() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRoleBinding",
        "metadata": {"name": "admins", "resourceVersion": "4", "uid": "u-admins"},
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": "view"},
        "subjects": [
            {"apiGroup": "rbac.authorization.k8s.io", "kind": "Group", "name": "ops"},
            {"apiGroup": "rbac.authorization.k8s.io", "kind": "User", "name": "ana"},
        ],
    }))
    .unwrap()
}

fn cluster_role(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
        "metadata": {"name": name},
        "rules": [{"apiGroups": [""], "resources": ["pods"], "verbs": ["get", "list"]}],
    }))
    .unwrap()
}

fn start(cx: &mut TestAppContext) -> App {
    let ports = TestPorts::seeded();
    let cluster = ports.connector.ports_for(&TestPorts::cluster_id());
    cluster.discovery.set_kinds(core_kinds());
    cluster.resources.insert(cluster_role_binding());
    cluster.resources.insert(cluster_role("view"));
    cluster.resources.insert(cluster_role("cluster-admin"));
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    app
}

fn dialog(app: &mut App) -> Option<gpui::Entity<BindingDialog>> {
    let ws = app.tab_workspace();
    app.vcx.update(|_, cx| {
        ws.read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<BindingDialog>()
    })
}

/// Opens the table behind the sidebar entry `id`, as a click on it does.
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
            CommandId::BINDING_OPEN_DIALOG,
            "oxikube_rbac_ui::binding",
            "k8s.binding_open_dialog",
        ),
        (
            CommandId::BINDING_CREATE,
            "oxikube_app::binding_ops",
            "k8s.binding_create",
        ),
        (
            CommandId::BINDING_EDIT,
            "oxikube_app::binding_ops",
            "k8s.binding_edit",
        ),
        (
            CommandId::BINDING_DELETE,
            "oxikube_app::binding_ops",
            "k8s.binding_delete",
        ),
    ] {
        assert_eq!(bus.owner(id), Some(owner), "{id}");
        assert_eq!(id.tool_name(), tool);
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
}

#[gpui::test]
fn access_control_reaches_the_cluster_role_bindings_table_with_role_ref_and_subjects(
    cx: &mut TestAppContext,
) {
    let mut app = start(cx);
    open_entry(&mut app, "access-control/clusterrolebindings");
    assert_eq!(app.tables(), ["ClusterRoleBinding"]);
    let ws = app.tab_workspace();
    let table = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
    let (columns, cells) = app.vcx.update(|_, cx| {
        let table = table.read(cx);
        let columns = table.read_rows(cx, |d| {
            d.layout()
                .visible_ids()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        });
        let titles = table.read_rows(cx, |d| {
            (0..d.layout().visible_len())
                .filter_map(|ix| d.layout().visible(ix).map(|c| c.title.to_string()))
                .collect::<Vec<_>>()
        });
        (columns, titles)
    });
    assert!(columns.contains(&"role".to_owned()), "{columns:?}");
    assert!(columns.contains(&"subjects".to_owned()), "{columns:?}");
    assert!(cells.contains(&"Role ref".to_owned()), "{cells:?}");
}

#[gpui::test]
fn a_binding_detail_shows_its_subjects_and_the_rules_of_the_role(cx: &mut TestAppContext) {
    let mut app = start(cx);
    open_entry(&mut app, "access-control/clusterrolebindings");
    app.click("td-0-0");
    app.press("enter");
    app.tick();
    app.tick();
    assert!(
        app.drawn("binding-panel"),
        "the binding panel is in the drawer"
    );
    assert!(app.drawn("binding-subject-row-1"));
    let view = drawer(&mut app);
    let panel = app
        .vcx
        .update(|_, cx| view.read(cx).kind_panel().cloned())
        .and_then(|panel| panel.downcast::<BindingPanel>().ok())
        .expect("the drawer's panel is the binding panel");
    let (subjects, rules) = app.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        (panel.subjects().to_vec(), panel.rules().to_vec())
    });
    assert_eq!(subjects.len(), 2);
    assert_eq!(&*subjects[0].name, "ops");
    assert_eq!(&*rules[0].resources, "pods");
    assert!(app.drawn("binding-rule-0"));
}

#[gpui::test]
fn new_on_the_role_bindings_table_opens_the_dialog(cx: &mut TestAppContext) {
    let mut app = start(cx);
    open_entry(&mut app, "access-control/rolebindings");
    assert_eq!(app.tables(), ["RoleBinding"]);
    app.click("resource-table-create");
    app.tick();
    assert!(dialog(&mut app).is_some(), "the binding dialog is open");
    assert!(app.drawn("binding-dialog"));
    assert!(
        app.drawn("binding-namespace"),
        "a RoleBinding has a namespace"
    );
}

#[gpui::test]
fn new_on_the_cluster_role_bindings_table_has_no_namespace(cx: &mut TestAppContext) {
    let mut app = start(cx);
    open_entry(&mut app, "access-control/clusterrolebindings");
    app.click("resource-table-create");
    app.tick();
    assert!(dialog(&mut app).is_some());
    assert!(
        !app.drawn("binding-namespace"),
        "a ClusterRoleBinding has none"
    );
}

#[gpui::test]
fn saving_a_binding_to_cluster_admin_asks_to_confirm_with_a_warning_then_writes(
    cx: &mut TestAppContext,
) {
    let mut app = start(cx);
    open_entry(&mut app, "access-control/clusterrolebindings");
    app.click("resource-table-create");
    app.tick();
    let dialog = dialog(&mut app).expect("the dialog");
    app.vcx.update(|window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.fill_name("ops-admins", window, cx);
            dialog.fill_subject(
                0,
                &SubjectDraft {
                    kind: SubjectKind::Group,
                    name: "ops".into(),
                    namespace: String::new(),
                },
                window,
                cx,
            );
        });
    });
    app.click("binding-role-pick-ClusterRole/cluster-admin");
    assert!(app.drawn("binding-escalation-0"), "the dialog warns");
    app.click("binding-save");
    app.tick();
    let resources = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    // Asked, with the guard's dry run (E19-S07) the only request sent.
    assert!(
        resources
            .mutating_calls()
            .iter()
            .all(oxikube_testkit::ResourceCall::is_dry_run),
        "asked, not written"
    );
    let ws = app.tab_workspace();
    let (title, message) = app.vcx.update(|_, cx| {
        let modal = ws
            .read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<ConfirmDialog>()
            .expect("the guard's confirmation");
        let modal = modal.read(cx);
        (modal.title().to_string(), modal.message_text().to_string())
    });
    assert_eq!(title, "Create the binding?");
    assert!(
        message.contains("ClusterRoleBinding ops-admins")
            && message.contains("ClusterRole/cluster-admin")
            && message.contains("WARNING: cluster-admin"),
        "{message}"
    );

    app.press("enter");
    app.tick();
    app.tick();
    let creates: Vec<bool> = resources
        .mutating_calls()
        .iter()
        .filter_map(|call| match call {
            ResourceCall::Create { options, .. } => Some(options.dry_run),
            _ => None,
        })
        .collect();
    assert_eq!(
        creates,
        [true, true, false],
        "the guard's preview and the handler's own dry run, then the create"
    );
    let audit = app.ports.state.audit_log();
    let record = audit.last().expect("an audit record");
    assert_eq!(&*record.cmd, "binding::Create");
    assert_eq!(
        record.detail.as_deref(),
        Some("role=ClusterRole/cluster-admin subjects=1")
    );
}

#[gpui::test]
fn a_read_only_cluster_refuses_the_save_and_says_so(cx: &mut TestAppContext) {
    let mut app = start(cx);
    open_entry(&mut app, "access-control/clusterrolebindings");
    app.click("resource-table-create");
    app.tick();
    let dialog = dialog(&mut app).expect("the dialog");
    app.vcx.update(|window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.fill_name("ops", window, cx);
            dialog.fill_subject(
                0,
                &SubjectDraft {
                    kind: SubjectKind::User,
                    name: "ana".into(),
                    namespace: String::new(),
                },
                window,
                cx,
            );
        });
    });
    app.click("binding-role-pick-ClusterRole/view");
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("the session is open");
    app.click("binding-save");
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
