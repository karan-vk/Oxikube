//! Rollout history and rollback on the real init path (E12-S06): `ctrl-l` on a Deployment's row
//! opens the history dialog with its revisions and the confirmation tier, its button reaches the
//! cluster through the app's own `CommandBus` and `MutationGuard` (a JSON patch of the Deployment's
//! template, rehearsed by the server first, audited with the revisions it went from and to), a
//! read-only cluster refuses, and the detail drawer's workload panel has the Rollback button.

use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_resources_ui::kinds::{RollbackDialog, RollbackStage};
use oxikube_testkit::{ResourceCall, TestPorts};
use serde_json::json;

use super::App;
use crate::app_state::AppState;

/// The fixture Deployment `demo/web` runs revision 2 (`registry.example/app:1.0`); this is the
/// ReplicaSet of `revision` owned by it.
fn replicaset(name: &str, revision: i64, image: &str, cause: Option<&str>) -> Resource {
    let mut annotations = json!({"deployment.kubernetes.io/revision": revision.to_string()});
    if let Some(cause) = cause {
        annotations["kubernetes.io/change-cause"] = json!(cause);
    }
    let deployment = oxikube_testkit::fixtures::deployment_ready();
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {
            "name": name, "namespace": "demo", "uid": format!("{name}-uid"),
            "creationTimestamp": "2026-01-01T00:00:00Z",
            "labels": {"app": "web", "pod-template-hash": "h"},
            "annotations": annotations,
            "ownerReferences": [{"apiVersion": "apps/v1", "kind": "Deployment", "name": "web",
                                 "uid": deployment.meta.uid.as_deref().unwrap_or(""),
                                 "controller": true}],
        },
        "spec": {"replicas": 0, "selector": {"matchLabels": {"app": "web"}},
                 "template": {
                     "metadata": {"labels": {"app": "web", "pod-template-hash": "h"}},
                     "spec": {"containers": [{"name": "app", "image": image}]},
                 }},
    }))
    .expect("a replicaset")
}

impl App {
    fn rollback_dialog(&mut self) -> Option<Entity<RollbackDialog>> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<RollbackDialog>()
        })
    }

    fn cluster_resources(&self) -> std::sync::Arc<oxikube_testkit::FakeResourcePort> {
        self.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources
    }

    fn seed_history(&self) {
        let resources = self.cluster_resources();
        resources.insert(replicaset(
            "web-1",
            1,
            "registry.example/app:0.9",
            Some("first release"),
        ));
        resources.insert(replicaset("web-2", 2, "registry.example/app:1.0", None));
    }
}

#[gpui::test]
fn rollback_is_on_the_bus_as_a_guarded_tool_stub_owned_by_the_workloads_service(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    assert_eq!(
        bus.owner(CommandId::WORKLOAD_ROLLBACK),
        Some("oxikube_app::workloads")
    );
    let tool = bus.tool(CommandId::WORKLOAD_ROLLBACK).expect("a stub");
    assert_eq!(tool.name.as_str(), "k8s.workload_rollback");
}

#[gpui::test]
fn ctrl_l_on_a_deployment_shows_its_history_and_the_button_rolls_back_through_the_guard(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_deployments_table();
    app.seed_history();
    app.press("ctrl-l");
    app.tick();
    let dialog = app.rollback_dialog().expect("ctrl-l opened the dialog");
    let (stage, selected, revisions) = app.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (
            d.stage(),
            d.selected(),
            d.rows()
                .iter()
                .map(|r| (r.revision, r.current))
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(stage, RollbackStage::Choosing);
    assert_eq!(selected, Some(1), "the previous revision");
    assert_eq!(revisions, [(2, true), (1, false)]);
    for part in [
        "rollback-dialog",
        "rollback-table",
        "rollback-tier",
        "rollback-confirm",
    ] {
        assert!(app.drawn(part), "{part}");
    }

    // The user confirms: the server rehearses the patch, then it is sent for real.
    let answer = oxikube_testkit::fixtures::deployment_ready();
    // The guard's dry run before it asks (E19-S07), the handler's, the patch.
    for _ in 0..3 {
        app.cluster_resources()
            .script()
            .patch
            .push_ok(answer.clone());
    }
    app.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.submit(cx)));
    app.tick();
    app.tick();
    assert!(app.rollback_dialog().is_none(), "the dialog closed");

    let writes = app.cluster_resources().mutating_calls();
    assert!(
        matches!(
            &writes[..],
            [
                ResourceCall::Patch { options: preview, .. },
                ResourceCall::Patch { options: dry, .. },
                ResourceCall::Patch { name, patch, options, .. },
            ] if preview.dry_run && dry.dry_run && !options.dry_run && name == "web"
                && patch.body[0]["path"] == "/spec/template"
                && patch.body[0]["value"]["spec"]["containers"][0]["image"]
                    == "registry.example/app:0.9"
        ),
        "{writes:?}"
    );
    let audit: Vec<_> = app
        .ports
        .state
        .audit_log()
        .into_iter()
        .filter(|r| !r.dry_run)
        .collect();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "workload::Rollback");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(
        audit[0].detail.as_deref(),
        Some("to_revision=1 from=2 to=1")
    );
    app.expect_toast("Rolled back Deployment demo/web from revision 2 to revision 1");
}

#[gpui::test]
fn a_read_only_cluster_refuses_the_key_and_sends_nothing(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_deployments_table();
    app.seed_history();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("a known cluster");
    app.press("ctrl-l");
    app.tick();
    assert!(app.rollback_dialog().is_none());
    assert!(app.cluster_resources().mutating_calls().is_empty());
    assert!(
        app.toasts().iter().any(|t| t.contains("read-only")),
        "{:?}",
        app.toasts()
    );
}

#[gpui::test]
fn the_detail_drawer_of_a_deployment_has_a_rollback_button_that_opens_the_dialog(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_deployments_table();
    app.seed_history();
    app.press("enter");
    app.tick();
    app.drawn("detail-view");
    app.tick();
    assert!(app.drawn("workload-rollback"), "the Rollback button");
    app.click("workload-rollback");
    app.tick();
    assert!(
        app.rollback_dialog().is_some(),
        "the button opens the dialog"
    );
}
