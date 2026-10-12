//! Bulk restart: `r` with several workloads selected, offered only for the kinds a template change
//! rolls, one confirmation, a patch and an audit record per workload.

use gpui::TestAppContext;
use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_ports::PatchKind;
use oxikube_testkit::ResourceCall;
use oxikube_workspace::command_surface::CommandSurface as _;

use super::*;
use crate::bulk::{BulkKind, BulkStage};

const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";

#[gpui::test]
fn r_with_three_deployments_selected_restarts_all_after_one_confirmation(cx: &mut TestAppContext) {
    let mut f = window(
        cx,
        [deployment("web"), deployment("api"), deployment("jobs")],
    );
    let table = f.open(deployments());
    select_all(&mut f, &table);
    assert_eq!(f.selected(&table).len(), 3);
    f.keys(&table, "r");
    f.settle();
    let d = dialog(&mut f).expect("the bulk dialog");
    // Nothing is sent until the one confirmation.
    assert!(f.ports().resources.mutating_calls().is_empty());
    assert!(drawn(&mut f, "bulk-warning"));
    with_dialog(&mut f, &d, |d, _, cx| {
        assert_eq!(d.kind(), BulkKind::Restart);
        assert_eq!(d.plan().summary().lines(5), ["3 Deployments in shop"]);
        d.confirm(cx);
    });
    f.settle();
    let report = d
        .read_with(&f.vcx, |d, _| d.report().cloned())
        .expect("results");
    assert_eq!(d.read_with(&f.vcx, |d, _| d.stage()), BulkStage::Done);
    assert_eq!(report.summary(), "Restarted 3 objects");

    // Per workload: the rehearsal, then the real merge patch stamping the template.
    let patches: Vec<_> = f
        .ports()
        .resources
        .mutating_calls()
        .into_iter()
        .map(|call| match call {
            ResourceCall::Patch {
                name,
                patch,
                options,
                ..
            } => (name, options.dry_run, patch.kind, patch.body),
            other => panic!("a patch: {other:?}"),
        })
        .collect();
    assert_eq!(patches.len(), 6);
    for (_, _, kind, body) in &patches {
        assert_eq!(*kind, PatchKind::Merge);
        assert!(body["spec"]["template"]["metadata"]["annotations"][RESTARTED_AT].is_string());
    }
    // One audit record per workload under one batch id, from the UI.
    let records = audit_of(&f, "workload::Restart");
    assert_eq!(records.len(), 3);
    for record in &records {
        assert_eq!(record.initiator, Initiator::Ui);
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
        let detail = record.detail.as_deref().unwrap();
        assert!(
            detail.starts_with(&format!("batch={} via=selection::Restart", report.batch)),
            "{detail}"
        );
    }
    assert_eq!(audit_of(&f, "selection::Restart").len(), 1);
}

#[gpui::test]
fn the_menu_and_the_palette_offer_restart_only_where_every_row_can_be_restarted(
    cx: &mut TestAppContext,
) {
    let mut f = window(cx, [deployment("web"), deployment("api")]);
    // Deployments: the menu lists "Restart 2 objects"; the palette gets selection::Restart.
    let table = f.open(deployments());
    select_all(&mut f, &table);
    let labels: Vec<_> = f
        .vcx
        .update(|_, cx| table.read(cx).action_entries(cx))
        .into_iter()
        .map(|e| e.label)
        .collect();
    assert!(
        labels.iter().any(|l| l == "Restart 2 objects"),
        "{labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l.starts_with("Set image")),
        "set image is one at a time"
    );
    let own = f.vcx.update(|_, cx| table.read(cx).own_commands(cx));
    assert!(own.contains(&CommandId::SELECTION_RESTART));

    // ReplicaSets have a pod template but are never restarted: neither the menu nor the palette.
    f.ports().resources.insert(
        Resource::from_json(json!({
            "apiVersion": "apps/v1", "kind": "ReplicaSet",
            "metadata": {"name": "web-5d", "namespace": "shop", "uid": "rs-1"},
            "spec": {"template": {"spec": {"containers": [{"name": "w", "image": "w:1"}]}}},
        }))
        .unwrap(),
    );
    f.ports().resources.insert(
        Resource::from_json(json!({
            "apiVersion": "apps/v1", "kind": "ReplicaSet",
            "metadata": {"name": "api-5d", "namespace": "shop", "uid": "rs-2"},
            "spec": {"template": {"spec": {"containers": [{"name": "w", "image": "w:1"}]}}},
        }))
        .unwrap(),
    );
    let table = f.open(replicasets());
    select_all(&mut f, &table);
    let own = f.vcx.update(|_, cx| table.read(cx).own_commands(cx));
    assert!(!own.contains(&CommandId::SELECTION_RESTART));
    f.keys(&table, "r");
    assert!(dialog(&mut f).is_none(), "`r` is not bound for ReplicaSets");
    let ran = f.vcx.update(|window, cx| {
        let targets = table.read(cx).action_targets(cx);
        table.update(cx, |t, cx| {
            t.run_action(CommandId::SELECTION_RESTART, targets, window, cx)
        })
    });
    assert!(
        !ran,
        "the command is refused for a kind that cannot be restarted"
    );
    assert!(dialog(&mut f).is_none());
    assert!(f.ports().resources.mutating_calls().is_empty());
}

#[gpui::test]
fn a_read_only_cluster_greys_the_bulk_restart_and_sends_nothing(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment("web"), deployment("api")]);
    let table = f.open(deployments());
    select_all(&mut f, &table);
    f.sessions.set_read_only(&cluster(), true).unwrap();
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let restart = entries
        .iter()
        .find(|e| e.command() == CommandId::WORKLOAD_RESTART)
        .unwrap();
    assert!(!restart.is_enabled());
    f.keys(&table, "r");
    assert!(dialog(&mut f).is_none());
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts.iter().any(|t| t.message.contains("read-only")),
        "{toasts:?}"
    );
    assert!(f.ports().resources.mutating_calls().is_empty());
}

#[gpui::test]
fn one_workload_failing_leaves_the_others_restarted(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment("web"), deployment("api")]);
    // The first rehearsal is refused.
    f.ports()
        .resources
        .script()
        .patch
        .push_err(OxiError::forbidden("deployments.apps \"api\" is forbidden"));
    let table = f.open(deployments());
    select_all(&mut f, &table);
    f.keys(&table, "r");
    let d = dialog(&mut f).unwrap();
    with_dialog(&mut f, &d, |d, _, cx| d.confirm(cx));
    f.settle();
    let report = d.read_with(&f.vcx, |d, _| d.report().cloned()).unwrap();
    assert_eq!((report.succeeded(), report.failed()), (1, 1));
    assert_eq!(report.summary(), "Restarted 1 of 2 objects, 1 failed");
}
