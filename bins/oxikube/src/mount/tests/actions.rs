//! Row actions on the real init path (E07-S08): a resource table opened from the sidebar has the
//! delete key and the delete dialog, and what the dialog confirms reaches the cluster through the
//! app's own `CommandBus` and `MutationGuard`, audited.

use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId, Propagation};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_resources_ui::actions::DeleteDialog;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

fn pods_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "Pod"),
        preferred: true,
        plural: "pods".into(),
        singular: "pod".into(),
        short_names: vec!["po".into()],
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
        namespaced: true,
    }
}

impl App {
    /// A pods table in the connected cluster's tab, the first row under the cursor.
    pub(super) fn open_pods_table(&mut self) {
        self.serve([pods_kind()]);
        // The pods the cluster's connection serves (the seeded ones are the ports' own copy).
        self.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources
            .insert(oxikube_testkit::fixtures::pod_running());
        self.press("enter");
        self.tick();
        self.click("sidebar-entry-workloads/pods");
        self.tick();
        // The sidebar click left the focus in the sidebar: the keys go to the table once it has it.
        let ws = self.tab_workspace();
        let table = self
            .vcx
            .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
        self.vcx.update(|window, cx| {
            let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
            window.focus(&focus, cx);
        });
        self.vcx.run_until_parked();
        self.press("down");
    }

    fn dialog(&mut self) -> Option<gpui::Entity<DeleteDialog>> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DeleteDialog>()
        })
    }
}

#[gpui::test]
fn delete_on_a_table_runs_through_the_bus_the_guard_and_the_audit_log(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_pods_table();
    app.press("delete");
    let dialog = app.dialog().expect("the delete key opens the dialog");
    let target = app
        .vcx
        .update(|_, cx| dialog.read(cx).plan().items()[0].target.clone());
    assert_eq!(&*target.gvk.kind, "Pod");
    let resources = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    assert!(
        resources.mutating_calls().is_empty(),
        "opening it sends nothing"
    );

    app.vcx
        .update(|_, cx| dialog.update(cx, |dialog, cx| dialog.confirm(cx)));
    app.tick();
    app.tick();

    assert_eq!(
        resources.mutating_calls().len(),
        2,
        "a server dry run, then the delete"
    );
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(&*audit[0].cmd, "resource::Delete");
    assert_eq!(audit[0].target, target);
}

#[gpui::test]
fn a_read_only_cluster_gets_no_dialog_and_the_bus_refuses_an_agent_too(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_pods_table();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let cluster = TestPorts::cluster_id();
    state
        .services()
        .sessions
        .set_read_only(&cluster, true)
        .expect("the session is open");
    app.press("delete");
    assert!(app.dialog().is_none(), "read-only: no dialog");
    app.expect_toast("This cluster is read-only");

    // Whatever a stale menu or an agent sends, the guard refuses it before any request.
    let bus = state.command_bus().expect("the bus");
    let outcome = futures::executor::block_on(bus.dispatch(
        Command::ResourceDelete {
            target: ResourceRef::namespaced(
                cluster.clone(),
                Gvk::new("", "v1", "Pod"),
                "default",
                "web",
            ),
            propagation: Propagation::Background,
        },
        DispatchContext::new(Initiator::Agent, "agent"),
    ));
    assert!(
        matches!(outcome, Err(oxikube_app::DispatchError::ReadOnly { .. })),
        "{outcome:?}"
    );
    let resources = app.ports.connector.ports_for(&cluster).resources;
    assert!(resources.mutating_calls().is_empty());
    assert!(
        app.ports
            .state
            .audit_log()
            .iter()
            .all(|r| r.outcome == AuditOutcome::Denied)
    );
    // The command is on the bus with its tool stub, owned by the app's actions.
    assert_eq!(
        bus.owner(CommandId::RESOURCE_DELETE),
        Some("oxikube_app::actions")
    );
    assert!(bus.tool(CommandId::RESOURCE_DELETE).is_some());
}
