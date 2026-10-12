//! The bulk actions on the real init path (E12-S19): a table opened from the sidebar, several rows
//! selected, and delete (`delete`), restart (`r`) and edit labels (`shift-l`) open the bulk dialog,
//! whose button sends one `selection::*` command through the app's own `CommandBus` and
//! `MutationGuard`: one confirmation, a result and an audit record per object under a shared batch
//! id, and a read-only cluster refusing the whole batch.

use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_domain::selection::LabelEdit;
use oxikube_ports::PatchKind;
use oxikube_resources_ui::bulk::{BulkDialog, BulkKind, BulkStage};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::{ResourceCall, TestPorts};
use serde_json::json;

use super::App;
use crate::app_state::AppState;

/// The platform's select-all chord.
const SELECT_ALL: &str = if cfg!(target_os = "macos") {
    "cmd-a"
} else {
    "ctrl-a"
};

fn served(group: &str, kind: &str, plural: &str) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, "v1", kind),
        preferred: true,
        plural: plural.into(),
        singular: kind.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete", "patch"]),
        namespaced: true,
    }
}

fn pod_named(name: &str) -> Resource {
    let mut pod = oxikube_testkit::fixtures::pod_running().to_value();
    pod["metadata"]["name"] = json!(name);
    pod["metadata"]["uid"] = json!(format!("{name}-uid"));
    pod["metadata"]["labels"] = json!({"app": "web", "tier": "old"});
    Resource::from_json(pod).expect("a pod")
}

fn deployment_named(name: &str) -> Resource {
    let mut deployment = oxikube_testkit::fixtures::deployment_ready().to_value();
    deployment["metadata"]["name"] = json!(name);
    deployment["metadata"]["uid"] = json!(format!("{name}-uid"));
    Resource::from_json(deployment).expect("a deployment")
}

impl App {
    /// A table of `kind` in the connected cluster's tab with every row selected.
    fn open_table_selecting_all(
        &mut self,
        kind: ResourceKind,
        sidebar: &'static str,
        objects: Vec<Resource>,
    ) -> Entity<ResourceTable> {
        self.serve([kind]);
        let resources = self
            .ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources;
        for object in objects {
            resources.insert(object);
        }
        self.press("enter");
        self.tick();
        self.click(sidebar);
        self.tick();
        let ws = self.tab_workspace();
        let table = self
            .vcx
            .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
        self.vcx.update(|window, cx| {
            let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
            window.focus(&focus, cx);
        });
        self.vcx.run_until_parked();
        self.press(SELECT_ALL);
        table
    }

    fn bulk_dialog(&mut self) -> Option<Entity<BulkDialog>> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<BulkDialog>()
        })
    }

    fn cluster_writes(&self) -> Vec<ResourceCall> {
        self.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources
            .mutating_calls()
    }
}

#[gpui::test]
fn the_bulk_commands_are_on_the_bus_as_guarded_tool_stubs_and_label_is_a_row_action(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    let offered: Vec<_> = crate::mount::logs::row_actions()
        .specs()
        .iter()
        .map(|spec| spec.command)
        .collect();
    for (id, tool) in [
        (CommandId::SELECTION_DELETE, "k8s.selection_delete"),
        (CommandId::SELECTION_RESTART, "k8s.selection_restart"),
        (CommandId::SELECTION_LABEL, "k8s.selection_label"),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_app::selection"), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
    assert!(offered.contains(&CommandId::SELECTION_LABEL), "{offered:?}");
}

#[gpui::test]
fn delete_with_five_pods_selected_asks_once_and_audits_each_under_one_batch(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    let pods: Vec<_> = (0..5).map(|i| pod_named(&format!("web-{i}"))).collect();
    app.open_table_selecting_all(
        served("", "Pod", "pods"),
        "sidebar-entry-workloads/pods",
        pods,
    );
    app.press("delete");
    let dialog = app
        .bulk_dialog()
        .expect("the delete key opens the bulk dialog");
    assert!(app.drawn("bulk-dialog") && app.drawn("bulk-what"));
    app.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        assert_eq!(d.kind(), BulkKind::Delete);
        // The seeded `pod_running` is one of the five names' neighbours: the table lists the
        // cluster's pods, and the selection is all of them.
        assert!(d.plan().targets().len() >= 5);
    });
    assert!(app.cluster_writes().is_empty(), "opening sends nothing");

    app.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.confirm(cx)));
    app.tick();
    app.tick();

    let (stage, report) = app
        .vcx
        .update(|_, cx| (dialog.read(cx).stage(), dialog.read(cx).report().cloned()));
    assert_eq!(stage, BulkStage::Done);
    let report = report.expect("a result per object");
    assert!(report.rows.len() >= 5);
    assert_eq!(report.failed(), 0, "{report:?}");
    assert_eq!(
        app.cluster_writes().len(),
        report.rows.len() * 2,
        "a server dry run, then the delete, for each"
    );
    let audit = app.ports.state.audit_log();
    let per_pod: Vec<_> = audit.iter().filter(|r| &*r.cmd == "pod::Delete").collect();
    assert_eq!(per_pod.len(), report.rows.len());
    for record in &per_pod {
        assert_eq!(record.initiator, Initiator::Ui);
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
        assert_eq!(
            record.detail.as_deref(),
            Some(format!("batch={} via=selection::Delete", report.batch).as_str())
        );
    }
    assert_eq!(
        audit
            .iter()
            .filter(|r| &*r.cmd == "selection::Delete")
            .count(),
        1,
        "the command itself is one more record"
    );
    assert!(app.drawn("bulk-results") && app.drawn("bulk-close"));
}

#[gpui::test]
fn shift_l_edits_the_labels_of_the_selection_through_the_guard(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let pods: Vec<_> = (0..3).map(|i| pod_named(&format!("web-{i}"))).collect();
    app.open_table_selecting_all(
        served("", "Pod", "pods"),
        "sidebar-entry-workloads/pods",
        pods,
    );
    app.press("shift-l");
    let dialog = app.bulk_dialog().expect("shift-l opens the label editor");
    assert!(app.drawn("bulk-field-switch") && !app.drawn("bulk-preview"));
    app.vcx.update(|window, cx| {
        dialog.update(cx, |d, cx| {
            d.editor_mut()
                .expect("the editor")
                .fill(0, "tier", "backend", window, cx);
        })
    });
    app.tick();
    assert!(app.drawn("bulk-preview"), "a sample shows the result");
    let edits = app
        .vcx
        .update(|_, cx| dialog.read(cx).editor().unwrap().edits(cx));
    assert_eq!(edits, [LabelEdit::set("tier", "backend")]);

    app.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.confirm(cx)));
    app.tick();
    app.tick();

    let writes = app.cluster_writes();
    assert!(!writes.is_empty());
    for write in &writes {
        let ResourceCall::Patch { patch, .. } = write else {
            panic!("a patch: {write:?}");
        };
        assert_eq!(patch.kind, PatchKind::Merge);
        assert_eq!(
            patch.body,
            json!({"metadata": {"labels": {"tier": "backend"}}})
        );
    }
    let audit = app.ports.state.audit_log();
    assert!(audit.iter().any(|r| &*r.cmd == "selection::Label"));
    assert!(audit.iter().all(|r| !format!("{r:?}").contains("backend")));
}

#[gpui::test]
fn r_with_two_deployments_selected_restarts_both_after_one_confirmation(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let deployments = vec![deployment_named("web"), deployment_named("api")];
    app.open_table_selecting_all(
        served("apps", "Deployment", "deployments"),
        "sidebar-entry-workloads/deployments",
        deployments,
    );
    app.press("r");
    let dialog = app
        .bulk_dialog()
        .expect("r opens the bulk dialog for several rows");
    app.vcx
        .update(|_, cx| assert_eq!(dialog.read(cx).kind(), BulkKind::Restart));
    assert!(app.cluster_writes().is_empty());
    app.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.confirm(cx)));
    app.tick();
    app.tick();
    let report = app
        .vcx
        .update(|_, cx| dialog.read(cx).report().cloned())
        .expect("results");
    assert!(report.rows.len() >= 2 && report.failed() == 0, "{report:?}");
    let restarts = app
        .ports
        .state
        .audit_log()
        .into_iter()
        .filter(|r| &*r.cmd == "workload::Restart")
        .count();
    assert_eq!(restarts, report.rows.len());
}

#[gpui::test]
fn a_read_only_cluster_refuses_every_bulk_action_and_sends_nothing(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let pods: Vec<_> = (0..3).map(|i| pod_named(&format!("web-{i}"))).collect();
    app.open_table_selecting_all(
        served("", "Pod", "pods"),
        "sidebar-entry-workloads/pods",
        pods,
    );
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("a known cluster");
    app.press("delete");
    app.press("shift-l");
    assert!(app.bulk_dialog().is_none(), "no dialog opens");
    assert!(
        app.toasts().iter().any(|t| t.contains("read-only")),
        "{:?}",
        app.toasts()
    );
    // The same refusal for the palette and an agent: the guard blocks every initiator, and the
    // whole batch before a request.
    let bus = state.command_bus().expect("the bus");
    for initiator in Initiator::ALL {
        let target = oxikube_domain::ids::ResourceRef::namespaced(
            TestPorts::cluster_id(),
            Gvk::new("", "v1", "Pod"),
            "demo",
            "web-0",
        );
        let command = oxikube_domain::command::Command::SelectionDelete {
            targets: vec![target],
            propagation: Default::default(),
        };
        let ctx = oxikube_app::command_bus::DispatchContext::new(initiator, "someone");
        let error = futures::executor::block_on(bus.dispatch(command, ctx)).unwrap_err();
        assert!(
            matches!(
                error,
                oxikube_app::command_bus::DispatchError::ReadOnly { .. }
            ),
            "{initiator}"
        );
    }
    assert!(app.cluster_writes().is_empty());
}
