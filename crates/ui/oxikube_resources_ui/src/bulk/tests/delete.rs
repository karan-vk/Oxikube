//! Bulk delete from the table: one confirmation for the selection at the highest tier, a result
//! per object, and the E07 dialog kept for one object and for Namespaces.

use gpui::TestAppContext;
use oxikube_app::BulkStatus;
use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{CommandId, Propagation};
use oxikube_domain::safety::ConfirmTier;
use oxikube_ports::DeleteOutcome;

use super::*;
use crate::bulk::{BulkKind, BulkStage};

#[gpui::test]
fn five_selected_pods_one_aggregated_confirmation_a_result_per_object(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(pods(5));
    let table = f.open_pods();
    assert_eq!(f.names(&table).len(), 5);
    // Two requests per object (the rehearsal, then the delete), in selection order: web-1 is
    // forbidden, web-3 is gone already.
    let ports = f.ports();
    let script = ports.resources.script();
    script.delete.push_ok(DeleteOutcome::Deleted); // web-0 rehearsal
    script.delete.push_ok(DeleteOutcome::Deleted); // web-0
    script
        .delete
        .push_err(OxiError::forbidden("pods \"web-1\" is forbidden")); // web-1 rehearsal
    script.delete.push_ok(DeleteOutcome::Deleted); // web-2 rehearsal
    script.delete.push_ok(DeleteOutcome::Deleted); // web-2
    script
        .delete
        .push_err(OxiError::not_found("pods \"web-3\" not found")); // web-3 rehearsal
    script.delete.push_ok(DeleteOutcome::Deleted); // web-4 rehearsal
    script.delete.push_ok(DeleteOutcome::Deleted); // web-4

    select_all(&mut f, &table);
    assert_eq!(f.selected(&table).len(), 5);
    f.keys(&table, "delete");
    let d = dialog(&mut f).expect("one bulk dialog for the whole selection");
    assert!(
        crate::actions::tests::dialog(&mut f).is_none(),
        "not the single-object dialog"
    );
    // The aggregated confirmation: counts by kind and namespace, no typing for pods.
    assert!(drawn(&mut f, "bulk-what"));
    with_dialog(&mut f, &d, |d, _, cx| {
        assert_eq!(d.kind(), BulkKind::Delete);
        assert_eq!(d.plan().targets().len(), 5);
        assert_eq!(d.plan().summary().lines(5), ["5 Pods in x"]);
        assert_eq!(d.plan().tier(), ConfirmTier::Simple);
        assert!(d.can_confirm(cx));
        d.confirm(cx);
    });
    f.settle();

    let (stage, report) = d.read_with(&f.vcx, |d, _| (d.stage(), d.report().cloned()));
    assert_eq!(stage, BulkStage::Done);
    let report = report.expect("results");
    let statuses: Vec<_> = report.rows.iter().map(|r| r.status.clone()).collect();
    assert_eq!(
        statuses,
        [
            BulkStatus::Ok("Deleted".into()),
            BulkStatus::Failed("pods \"web-1\" is forbidden".into()),
            BulkStatus::Ok("Deleted".into()),
            BulkStatus::Skipped("already gone".into()),
            BulkStatus::Ok("Deleted".into()),
        ]
    );
    assert_eq!(
        report.summary(),
        "Deleted 3 of 5 objects, 1 skipped, 1 failed"
    );

    // One audit record per object under one batch id, all from the UI, plus the command's own.
    let per_pod = audit_of(&f, "pod::Delete");
    assert_eq!(per_pod.len(), 5);
    assert!(per_pod.iter().all(|r| r.initiator == Initiator::Ui));
    assert!(per_pod.iter().all(|r| {
        r.detail.as_deref()
            == Some(format!("batch={} via=selection::Delete", report.batch).as_str())
    }));
    assert_eq!(
        per_pod
            .iter()
            .filter(|r| r.outcome == AuditOutcome::Succeeded)
            .count(),
        4,
        "a skipped row is not a failure"
    );
    assert_eq!(audit_of(&f, "selection::Delete").len(), 1);

    // The dialog stays up to show the list, with Retry and Close buttons.
    assert!(drawn(&mut f, "bulk-summary"));
    assert!(drawn(&mut f, "bulk-results"));
    assert!(drawn(&mut f, "bulk-retry"));
    assert!(drawn(&mut f, "bulk-close"));
    with_dialog(&mut f, &d, |d, _, cx| d.cancel(cx));
    assert!(dialog(&mut f).is_none());
}

#[gpui::test]
fn one_row_and_a_namespace_keep_the_single_object_dialog(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(pods(3));
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.keys(&table, "delete");
    assert!(dialog(&mut f).is_none());
    assert!(
        crate::actions::tests::dialog(&mut f).is_some(),
        "the E07 dialog"
    );
}

#[gpui::test]
fn a_right_click_inside_the_selection_deletes_the_selection(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(pods(4));
    let table = f.open_pods();
    select_all(&mut f, &table);
    crate::actions::tests::right_click(&mut f, 2);
    let labels: Vec<_> = f
        .vcx
        .update(|_, cx| table.read(cx).action_entries(cx))
        .into_iter()
        .map(|e| e.label)
        .collect();
    // Open, Copy Name, Select All, Edit labels, Delete: per-kind actions are for one object only.
    assert!(labels.iter().any(|l| l == "Delete 4 objects"), "{labels:?}");
    assert!(
        labels.iter().any(|l| l == "Edit labels 4 objects"),
        "{labels:?}"
    );
    let delete = labels.iter().position(|l| l == "Delete 4 objects").unwrap();
    // The menu lists Open, Copy Name and Select All before the actions.
    crate::actions::tests::choose(&mut f, 3 + delete);
    let d = dialog(&mut f).expect("the bulk dialog");
    assert_eq!(d.read_with(&f.vcx, |d, _| d.plan().targets().len()), 4);
}

#[gpui::test]
fn a_read_only_cluster_refuses_the_whole_batch_before_any_request(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(pods(3));
    let table = f.open_pods();
    select_all(&mut f, &table);
    f.keys(&table, "delete");
    let d = dialog(&mut f).expect("dialog");
    // Read-only goes on while the dialog is open: the guard refuses the batch as a whole.
    f.sessions.set_read_only(&cluster(), true).unwrap();
    with_dialog(&mut f, &d, |d, _, cx| d.confirm(cx));
    f.settle();
    let (stage, report, error) = d.read_with(&f.vcx, |d, _| {
        (d.stage(), d.report().cloned(), d.error().map(str::to_owned))
    });
    assert_eq!(
        stage,
        BulkStage::Confirm,
        "back to the question, nothing ran"
    );
    assert!(report.is_none());
    assert!(error.unwrap().contains("read-only"));
    assert!(f.ports().resources.mutating_calls().is_empty());
    let audit = f.state.audit_log();
    assert_eq!(
        audit.len(),
        1,
        "the one command is audited as denied: {audit:?}"
    );
    assert_eq!(audit[0].outcome, AuditOutcome::Denied);

    // And with the cluster read-only to begin with, the key says why instead of opening it.
    with_dialog(&mut f, &d, |d, _, cx| d.cancel(cx));
    f.keys(&table, "delete");
    assert!(dialog(&mut f).is_none());
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts.iter().any(|t| t.message.contains("read-only")),
        "{toasts:?}"
    );
}

/// The command palette (E11-S03) hands the bulk command to the focused table, which opens the one
/// dialog for the selection: not one guarded command (and one confirmation, each replacing the
/// last) per object.
#[gpui::test]
fn the_palette_runs_delete_selection_through_the_one_dialog(cx: &mut TestAppContext) {
    use oxikube_workspace::command_surface::{focused, run_on_focused};

    let mut f = Fixture::with_actions(cx);
    f.connect_with(pods(3));
    let table = f.open_pods();
    select_all(&mut f, &table);

    let snapshot = f
        .vcx
        .update(|window, cx| focused(window, cx))
        .expect("the focused table is a command surface");
    for command in [
        CommandId::POD_DELETE,
        CommandId::SELECTION_DELETE,
        CommandId::SELECTION_LABEL,
    ] {
        assert!(snapshot.own_commands.contains(&command), "{command}");
    }
    assert!(
        !snapshot
            .own_commands
            .contains(&CommandId::SELECTION_RESTART),
        "pods cannot be restarted"
    );
    assert_eq!(snapshot.target.targets.len(), 3);

    let targets = snapshot.target.targets;
    let ran = f
        .vcx
        .update(|window, cx| run_on_focused(CommandId::SELECTION_DELETE, targets, window, cx));
    f.settle();
    assert!(ran, "the table took the command");

    let d = dialog(&mut f).expect("one dialog for the whole selection");
    assert_eq!(d.read_with(&f.vcx, |d, _| d.plan().targets().len()), 3);
    assert!(
        f.ports().resources.mutating_calls().is_empty(),
        "nothing is deleted before the dialog is confirmed"
    );
    assert!(f.state.audit_log().is_empty());
}

#[gpui::test]
fn a_cascading_choice_raises_the_tier_to_typing_the_cluster_name(cx: &mut TestAppContext) {
    let mut f = window(cx, [deployment("web"), deployment("api")]);
    let table = f.open(deployments());
    select_all(&mut f, &table);
    f.keys(&table, "delete");
    let d = dialog(&mut f).expect("dialog");
    // Deployments own dependents, so the dialog asks how to treat them.
    assert!(drawn(&mut f, "bulk-propagation"));
    with_dialog(&mut f, &d, |d, _, cx| {
        assert_eq!(d.plan().tier(), ConfirmTier::Simple);
        assert!(!d.plan().targets().is_empty());
        d.set_propagation(Propagation::Foreground, cx);
    });
    let (tier, phrase) = d.read_with(&f.vcx, |d, _| {
        (d.plan().tier(), d.plan().phrase().map(str::to_owned))
    });
    assert_eq!(tier, ConfirmTier::TypeName);
    let phrase = phrase.expect("the cluster name is asked for");
    assert!(drawn(&mut f, "bulk-type-input"));
    // Not enabled until it is typed; a wrong text sends nothing.
    assert!(!d.read_with(&f.vcx, |d, cx| d.can_confirm(cx)));
    with_dialog(&mut f, &d, |d, window, cx| d.type_text("nope", window, cx));
    assert!(!d.read_with(&f.vcx, |d, cx| d.can_confirm(cx)));
    with_dialog(&mut f, &d, |d, _, cx| d.confirm(cx));
    assert!(f.ports().resources.mutating_calls().is_empty());
    with_dialog(&mut f, &d, |d, window, cx| d.type_text(&phrase, window, cx));
    assert!(d.read_with(&f.vcx, |d, cx| d.can_confirm(cx)));
    with_dialog(&mut f, &d, |d, _, cx| d.confirm(cx));
    f.settle();
    let report = d
        .read_with(&f.vcx, |d, _| d.report().cloned())
        .expect("results");
    assert_eq!(report.succeeded(), 2);
    // The delete went out with foreground propagation.
    assert!(
        f.ports()
            .resources
            .mutating_calls()
            .iter()
            .any(|call| matches!(
                call,
                oxikube_testkit::ResourceCall::Delete { options, .. }
                    if options.propagation == Some(oxikube_ports::PropagationPolicy::Foreground)
            ))
    );
}

#[gpui::test]
fn cancel_sends_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(pods(3));
    let table = f.open_pods();
    select_all(&mut f, &table);
    f.keys(&table, "delete");
    let d = dialog(&mut f).unwrap();
    with_dialog(&mut f, &d, |d, _, cx| d.cancel(cx));
    assert!(dialog(&mut f).is_none());
    assert!(f.ports().resources.mutating_calls().is_empty());
    assert!(f.state.audit_log().is_empty());
}
