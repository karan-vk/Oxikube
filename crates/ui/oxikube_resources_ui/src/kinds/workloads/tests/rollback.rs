//! Rollback (E12-S06): the row action, the `ctrl-l` key, the dialog with the rollout history and
//! its confirmation tier, and the run through the real bus and guard (blocked read-only, audited,
//! medium).

use futures::executor::block_on;
use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::{Command, CommandId};
use oxikube_testkit::ResourceCall;
use serde_json::{Value, json};

use super::{daemonsets, deployments, replicasets, replication_controllers, statefulsets};
use crate::kinds::{RollbackDialog, RollbackStage};
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

fn template(image: &str, hash: Option<&str>) -> Value {
    let mut labels = json!({"app": "web"});
    if let Some(hash) = hash {
        labels["pod-template-hash"] = json!(hash);
    }
    json!({
        "metadata": {"labels": labels, "creationTimestamp": null},
        "spec": {"containers": [{"name": "app", "image": image}]},
    })
}

/// The Deployment `shop/web` at revision 3 running `web:3`.
fn deployment(paused: bool) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "web", "namespace": "shop", "uid": "dep-uid",
                     "resourceVersion": "10", "creationTimestamp": "2026-01-01T00:00:00Z",
                     "annotations": {"deployment.kubernetes.io/revision": "3"}},
        "spec": {"replicas": 3, "paused": paused, "selector": {"matchLabels": {"app": "web"}},
                 "template": template("web:3", None)},
        "status": {"replicas": 3, "readyReplicas": 3}
    }))
    .unwrap()
}

fn replicaset(name: &str, revision: i64, image: &str, cause: Option<&str>) -> Resource {
    let mut annotations = json!({"deployment.kubernetes.io/revision": revision.to_string()});
    if let Some(cause) = cause {
        annotations["kubernetes.io/change-cause"] = json!(cause);
    }
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {"name": name, "namespace": "shop", "uid": format!("{name}-uid"),
                     "resourceVersion": "3", "creationTimestamp": "2026-01-01T00:00:00Z",
                     "labels": {"app": "web", "pod-template-hash": "h"},
                     "annotations": annotations,
                     "ownerReferences": [{"apiVersion": "apps/v1", "kind": "Deployment",
                                          "name": "web", "uid": "dep-uid", "controller": true}]},
        "spec": {"replicas": if revision == 3 { 3 } else { 0 },
                 "selector": {"matchLabels": {"app": "web"}},
                 "template": template(image, Some("h"))},
        "status": {"replicas": 3, "readyReplicas": if revision == 3 { 3 } else { 0 }}
    }))
    .unwrap()
}

/// Three revisions of `web` with change causes.
fn revisions() -> Vec<Resource> {
    vec![
        replicaset("web-1", 1, "web:1", None),
        replicaset("web-2", 2, "web:2", Some("bump to 2")),
        replicaset("web-3", 3, "web:3", Some("bump to 3")),
    ]
}

fn window(cx: &mut TestAppContext, objects: impl IntoIterator<Item = Resource>) -> Fixture {
    let mut f = Fixture::with_workloads(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.discovery.set_kinds([
        deployments(),
        statefulsets(),
        replicasets(),
        daemonsets(),
        replication_controllers(),
    ]);
    for object in objects {
        ports.resources.insert(object);
    }
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
    f
}

fn dialog(f: &mut Fixture) -> Option<Entity<RollbackDialog>> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let layer = tab.read(cx).workspace().read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<RollbackDialog>()
    })
}

fn drawn(f: &mut Fixture, selector: &'static str) -> bool {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    f.vcx.debug_bounds(selector).is_some()
}

fn labels(f: &mut Fixture, table: &Entity<ResourceTable>) -> Vec<String> {
    f.vcx
        .update(|_, cx| table.read(cx).action_entries(cx))
        .into_iter()
        .map(|entry| entry.label)
        .collect()
}

fn rollback_audit(f: &Fixture) -> Vec<(AuditOutcome, String)> {
    f.state
        .audit_log()
        .iter()
        .filter(|r| &*r.cmd == "workload::Rollback")
        .map(|r| (r.outcome, r.detail.as_deref().unwrap_or("").to_owned()))
        .collect()
}

/// Opens the deployments table, puts the cursor on the first row and presses `ctrl-l`.
fn press_ctrl_l(f: &mut Fixture) -> Entity<RollbackDialog> {
    let table = f.open(deployments());
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "ctrl-l");
    f.settle();
    dialog(f).expect("the rollback dialog is open")
}

fn confirm(f: &mut Fixture, dialog: &Entity<RollbackDialog>) {
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.submit(cx)));
    f.settle();
}

fn answer_patches(f: &Fixture) {
    f.ports()
        .resources
        .script()
        .patch
        .push_ok(deployment(false));
    f.ports()
        .resources
        .script()
        .patch
        .push_ok(deployment(false));
}

#[gpui::test]
fn rollback_is_offered_for_workloads_with_a_history_only(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment(false)]);
    for (kind, offered) in [
        (deployments(), true),
        (statefulsets(), true),
        (daemonsets(), true),
        (replicasets(), true),
        (replication_controllers(), false),
    ] {
        let name = kind.gvk.kind.to_string();
        let table = f.open(kind);
        let entries = labels(&mut f, &table);
        assert_eq!(
            entries.contains(&"Rollback".to_owned()),
            offered,
            "{name}: {entries:?}"
        );
    }
}

#[gpui::test]
fn ctrl_l_opens_the_history_with_the_previous_revision_chosen_and_the_tier_shown(
    cx: &mut TestAppContext,
) {
    let mut f = window(cx, [deployment(false)].into_iter().chain(revisions()));
    let dialog = press_ctrl_l(&mut f);
    assert!(f.dispatcher.sent().is_empty(), "opening sends nothing");
    assert!(f.ports().resources.mutating_calls().is_empty());

    let (stage, selected, rows) = f.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (d.stage(), d.selected(), d.rows().to_vec())
    });
    assert_eq!(stage, RollbackStage::Choosing);
    assert_eq!(selected, Some(2), "the previous revision");
    let table: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r.revision,
                r.current,
                r.cause.as_str(),
                r.images.as_str(),
                r.ready.as_str(),
            )
        })
        .collect();
    assert_eq!(
        table,
        [
            (3, true, "bump to 3", "web:3", "3/3"),
            (2, false, "bump to 2", "web:2", "0/0"),
            (1, false, "-", "web:1", "0/0"),
        ]
    );
    assert_eq!(
        rows[1]
            .changes
            .iter()
            .map(|c| c.path.as_str())
            .collect::<Vec<_>>(),
        ["spec.containers[app].image"]
    );

    for part in [
        "rollback-dialog",
        "rollback-subtitle",
        "rollback-table",
        "rollback-preview",
        "rollback-tier",
        "rollback-confirm",
    ] {
        assert!(drawn(&mut f, part), "{part} is drawn");
    }
    for row in ["rollback-row-1", "rollback-row-2", "rollback-row-3"] {
        assert!(drawn(&mut f, row), "{row}");
    }
    assert!(
        drawn(&mut f, "rollback-current-3"),
        "the running revision is marked"
    );
    // The tier is the command's declared risk, shown before anything is asked.
    let tier = RollbackDialog::tier_line();
    assert!(tier.starts_with("Risk: medium."), "{tier}");
    assert!(!drawn(&mut f, "rollback-paused"));
}

#[gpui::test]
fn the_dialog_is_up_and_loading_before_the_history_answers(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment(false)].into_iter().chain(revisions()));
    let table = f.open(deployments());
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.vcx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
        window.focus(&focus, cx);
    });
    f.vcx.run_until_parked();
    f.vcx.update(|window, cx| {
        window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-l").expect("a key"), cx);
    });
    let dialog = dialog(&mut f).expect("open before the read answers");
    let (stage, rows) = f
        .vcx
        .update(|_, cx| (dialog.read(cx).stage(), dialog.read(cx).rows().len()));
    assert_eq!((stage, rows), (RollbackStage::Loading, 0));
    assert!(drawn(&mut f, "rollback-loading"));
    f.settle();
    let stage = f.vcx.update(|_, cx| dialog.read(cx).stage());
    assert_eq!(stage, RollbackStage::Choosing);
    assert!(!drawn(&mut f, "rollback-loading"));
}

#[gpui::test]
fn choosing_a_row_changes_the_preview_and_the_running_revision_cannot_be_chosen_to_run(
    cx: &mut TestAppContext,
) {
    let mut f = window(cx, [deployment(false)].into_iter().chain(revisions()));
    let dialog = press_ctrl_l(&mut f);
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.select(1, cx)));
    let (selected, can) = f
        .vcx
        .update(|_, cx| (dialog.read(cx).selected(), dialog.read(cx).can_submit()));
    assert_eq!((selected, can), (Some(1), true));
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.move_selection(-1, cx)));
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.move_selection(-5, cx)));
    let selected = f.vcx.update(|_, cx| dialog.read(cx).selected());
    assert_eq!(selected, Some(3), "up stops at the newest row");
    let can = f.vcx.update(|_, cx| dialog.read(cx).can_submit());
    assert!(!can, "the running revision is not a rollback");
    // Rows that are not in the table are ignored.
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.select(42, cx)));
    assert_eq!(f.vcx.update(|_, cx| dialog.read(cx).selected()), Some(3));
}

#[gpui::test]
fn confirming_rolls_back_through_the_guard_audits_from_and_to_and_closes(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment(false)].into_iter().chain(revisions()));
    let dialog = press_ctrl_l(&mut f);
    answer_patches(&f);
    confirm(&mut f, &dialog);

    assert!(dialog_closed(&mut f), "the dialog closed");
    let writes = f.ports().resources.mutating_calls();
    assert_eq!(
        writes.len(),
        2,
        "the server rehearses, then the real patch: {writes:?}"
    );
    assert!(matches!(
        &writes[1],
        ResourceCall::Patch { name, patch, options, .. }
            if name == "web" && !options.dry_run
                && patch.body[0]["value"]["spec"]["containers"][0]["image"] == "web:2"
    ));
    assert_eq!(
        rollback_audit(&f),
        [(
            AuditOutcome::Succeeded,
            "to_revision=2 from=3 to=2".to_owned()
        )]
    );
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts
            .iter()
            .any(|t| t.message.contains("from revision 3 to revision 2")),
        "{toasts:?}"
    );
}

fn dialog_closed(f: &mut Fixture) -> bool {
    dialog(f).is_none()
}

#[gpui::test]
fn a_paused_deployment_is_said_up_front_and_cannot_be_rolled_back(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment(true)].into_iter().chain(revisions()));
    let dialog = press_ctrl_l(&mut f);
    assert!(drawn(&mut f, "rollback-paused"));
    assert!(f.vcx.update(|_, cx| dialog.read(cx).paused()));
    assert!(!f.vcx.update(|_, cx| dialog.read(cx).can_submit()));
    confirm(&mut f, &dialog);
    assert!(f.ports().resources.mutating_calls().is_empty());
    assert!(f.state.audit_log().is_empty(), "nothing reached the bus");
}

#[gpui::test]
fn a_refusal_stays_in_the_dialog_with_the_servers_message(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment(false)].into_iter().chain(revisions()));
    let dialog = press_ctrl_l(&mut f);
    f.ports()
        .resources
        .script()
        .patch
        .push_err(oxikube_domain::OxiError::forbidden(
            "admission webhook \"quota\" denied the request",
        ));
    confirm(&mut f, &dialog);
    let (stage, error) = f.vcx.update(|_, cx| {
        (
            dialog.read(cx).stage(),
            dialog.read(cx).error().map(str::to_owned),
        )
    });
    assert_eq!(stage, RollbackStage::Choosing, "back to the table");
    assert!(error.expect("a message").contains("quota"));
    assert!(drawn(&mut f, "rollback-error"));
    assert_eq!(rollback_audit(&f)[0].0, AuditOutcome::Failed);
}

#[gpui::test]
fn a_failed_history_read_shows_the_error_and_not_an_empty_history(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment(false)].into_iter().chain(revisions()));
    let table = f.open(deployments());
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.ports()
        .resources
        .script()
        .list
        .push_err(oxikube_domain::OxiError::forbidden(
            "replicasets is forbidden for this account",
        ));
    f.keys(&table, "ctrl-l");
    f.settle();
    let dialog = dialog(&mut f).expect("the rollback dialog is open");
    let (stage, error) = f.vcx.update(|_, cx| {
        (
            dialog.read(cx).stage(),
            dialog.read(cx).error().map(str::to_owned),
        )
    });
    assert_eq!(stage, RollbackStage::Choosing);
    assert!(error.expect("a message").contains("forbidden"));
    assert!(drawn(&mut f, "rollback-error"));
    assert!(
        !f.vcx.debug_bounds("rollback-empty").is_some(),
        "a failed read is not an empty history"
    );
    assert!(!f.vcx.update(|_, cx| dialog.read(cx).can_submit()));
}

#[gpui::test]
fn a_deployment_with_no_revisions_says_there_is_nothing_to_roll_back_to(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment(false)]);
    let dialog = press_ctrl_l(&mut f);
    assert!(f.vcx.update(|_, cx| dialog.read(cx).error().is_none()));
    assert!(drawn(&mut f, "rollback-empty"));
    assert!(!f.vcx.debug_bounds("rollback-error").is_some());
}

#[gpui::test]
fn cancelling_sends_nothing(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment(false)].into_iter().chain(revisions()));
    let dialog = press_ctrl_l(&mut f);
    f.vcx
        .update(|_, cx| dialog.update(cx, |d, cx| d.cancel(cx)));
    f.settle();
    assert!(dialog_closed(&mut f));
    assert!(f.state.audit_log().is_empty());
    assert!(f.ports().resources.mutating_calls().is_empty());
}

#[gpui::test]
fn a_read_only_cluster_greys_rollback_out_and_the_key_says_why(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment(false)].into_iter().chain(revisions()));
    let table = f.open(deployments());
    f.sessions.set_read_only(&cluster(), true).unwrap();
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let entry = entries
        .iter()
        .find(|e| e.command() == CommandId::WORKLOAD_ROLLBACK)
        .expect("listed");
    assert!(!entry.is_enabled());
    assert!(
        entry.reason().unwrap().contains("read-only"),
        "{:?}",
        entry.reason()
    );
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.keys(&table, "ctrl-l");
    assert!(dialog_closed(&mut f), "no dialog");
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts.iter().any(|t| t.message.contains("read-only")),
        "{toasts:?}"
    );
    assert!(f.ports().resources.mutating_calls().is_empty());
}

#[gpui::test]
fn on_a_replicaset_the_dialog_is_its_deployments_history_starting_on_that_revision(
    cx: &mut TestAppContext,
) {
    let mut f = window(cx, [deployment(false)].into_iter().chain(revisions()));
    let table = f.open(replicasets());
    // Rows sort by name: `web-1` first.
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.keys(&table, "ctrl-l");
    f.settle();
    let dialog = dialog(&mut f).expect("the dialog");
    let (selected, subtitle) = f.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (
            d.selected(),
            d.history().map(|h| h.workload.name.to_string()),
        )
    });
    assert_eq!(subtitle.as_deref(), Some("web"), "the Deployment's history");
    assert_eq!(selected, Some(1), "the ReplicaSet's own revision");
}

#[gpui::test]
fn ctrl_l_does_nothing_on_a_replication_controller(cx: &mut TestAppContext) {
    let mut f = window(cx, []);
    let table = f.open(replication_controllers());
    f.dispatcher.clear();
    f.keys(&table, "ctrl-l");
    assert!(dialog_closed(&mut f));
    assert!(
        f.dispatcher
            .sent()
            .iter()
            .all(|c| !matches!(c, Command::WorkloadRollback { .. }))
    );
}

#[gpui::test]
fn a_long_history_lists_the_newest_hundred_and_counts_the_rest(cx: &mut TestAppContext) {
    let many = (1..=120).map(|n| replicaset(&format!("web-{n}"), n, &format!("web:{n}"), None));
    let mut deployment = deployment(false).to_value();
    deployment["metadata"]["annotations"]["deployment.kubernetes.io/revision"] = json!("120");
    let mut f = window(
        cx,
        [Resource::from_json(deployment).unwrap()]
            .into_iter()
            .chain(many),
    );
    let dialog = press_ctrl_l(&mut f);
    let (rows, hidden, newest, selected) = f.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (
            d.rows().len(),
            d.hidden(),
            d.rows()[0].revision,
            d.selected(),
        )
    });
    assert_eq!(
        (rows, hidden, newest),
        (crate::kinds::workloads::rollback::MAX_ROWS, 20, 120)
    );
    assert_eq!(selected, Some(119));
    assert!(drawn(&mut f, "rollback-hidden"));
}

#[gpui::test]
fn a_replicaset_older_than_the_cap_is_still_listed_and_chosen(cx: &mut TestAppContext) {
    let many = (1..=150).map(|n| replicaset(&format!("web-{n:03}"), n, &format!("web:{n}"), None));
    let mut deployment = deployment(false).to_value();
    deployment["metadata"]["annotations"]["deployment.kubernetes.io/revision"] = json!("150");
    let mut f = window(
        cx,
        [Resource::from_json(deployment).unwrap()]
            .into_iter()
            .chain(many),
    );
    let table = f.open(replicasets());
    // Rows sort by name: `web-001` is the oldest revision, far outside the newest hundred.
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.keys(&table, "ctrl-l");
    f.settle();
    let dialog = dialog(&mut f).expect("the dialog");
    let (selected, rows, hidden) = f.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (d.selected(), d.rows().len(), d.hidden())
    });
    assert_eq!(
        selected,
        Some(1),
        "the ReplicaSet's own revision can be chosen"
    );
    assert_eq!((rows, hidden), (150, 0), "the table reaches back to it");
}
