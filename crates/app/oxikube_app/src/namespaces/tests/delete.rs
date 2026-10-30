//! `namespace::Delete`: irreversible, the name typed, a dry run then the delete, audited with
//! what the namespace held.

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_ports::DeleteOutcome;
use oxikube_testkit::ResourceCall;

use super::{delete, harness, namespace_ref};
use crate::command_bus::{DispatchError, Outcome};
use crate::guard::{Confirmation, ConfirmationError, DryRunPreview};
use crate::testing::{Harness, ctx};

fn deletes(h: &Harness) -> Vec<(bool, String)> {
    h.writes("a")
        .into_iter()
        .map(|call| match call {
            ResourceCall::Delete { options, name, .. } => (options.dry_run, name),
            other => panic!("unexpected write {other:?}"),
        })
        .collect()
}

#[test]
fn the_command_is_irreversible_and_needs_the_name() {
    let meta = oxikube_domain::command::lookup(CommandId::NAMESPACE_DELETE).unwrap();
    assert!(meta.mutating);
    assert_eq!(meta.risk, Some(Risk::Irreversible));
    assert_eq!(meta.confirm, ConfirmTier::TypeName);
    assert_eq!(
        CommandId::NAMESPACE_DELETE.tool_name(),
        "app.namespace_delete"
    );
    // The tool stub advertises the same worst case.
    assert_eq!(meta.tool_risk(), Some(Risk::Irreversible));
}

#[test]
fn the_name_is_typed_then_a_dry_run_and_the_delete_with_the_counts_audited() {
    let h = harness();
    h.connect("a", false);
    // The guard's dry run before each of the two asks, the handler's own, then the delete.
    h.allow_deletes("a", 4);
    let command = delete("a", "payments", &[("Pod", 12), ("Deployment", 3)]);

    let request = h.ask(command.clone(), ctx(Initiator::Ui));
    assert_eq!(request.tier, ConfirmTier::TypeName);
    assert_eq!(request.risk, Some(Risk::Irreversible));
    assert_eq!(request.expected_name.as_deref(), Some("payments"));
    let Some(DryRunPreview::Rehearsed(preview)) = &request.preview else {
        panic!("an irreversible delete is dry-run before it is asked: {request:?}");
    };
    assert_eq!(
        preview.message.as_deref(),
        Some("Dry run: namespace payments can be deleted")
    );
    assert_eq!(
        deletes(&h),
        [(true, "payments".to_owned())],
        "only the dry run went out before the confirmation"
    );

    // A wrong name is refused, and nothing is sent.
    let wrong = h.dispatch(
        command.clone(),
        ctx(Initiator::Ui).with_confirmation(Confirmation::typed(request.token, "payment")),
    );
    assert!(matches!(
        wrong,
        Err(DispatchError::Confirmation(
            ConfirmationError::NameMismatch { .. }
        ))
    ));
    assert_eq!(deletes(&h).len(), 1, "a wrong name sends nothing");

    let out = h.confirm_and_run(command, ctx(Initiator::Ui)).unwrap();
    let Outcome::Completed(output) = out else {
        panic!("expected completion");
    };
    assert_eq!(
        output.message.as_deref(),
        Some("Deleted namespace payments")
    );
    assert_eq!(
        deletes(&h),
        [
            (true, "payments".to_owned()),
            (true, "payments".to_owned()),
            (true, "payments".to_owned()),
            (false, "payments".to_owned())
        ]
    );

    let audit: Vec<_> = h
        .audit()
        .into_iter()
        .filter(|r| r.outcome == AuditOutcome::Succeeded && !r.dry_run)
        .collect();
    assert_eq!(audit.len(), 1);
    assert_eq!(&*audit[0].cmd, "namespace::Delete");
    assert_eq!(audit[0].target, namespace_ref("a", "payments"));
    assert_eq!(
        audit[0].detail.as_deref(),
        Some("contents=Deployment:3,Pod:12"),
        "the record says what went with it"
    );
}

#[test]
fn a_terminating_namespace_is_reported_as_deleting() {
    let h = harness();
    h.connect("a", false);
    let resources = h.resources("a");
    // The guard's dry run, the handler's, then the delete.
    resources.script().delete.push_ok(DeleteOutcome::Deleted);
    resources.script().delete.push_ok(DeleteOutcome::Deleted);
    resources.script().delete.push_ok(DeleteOutcome::Deleting(
        oxikube_domain::Resource::from_json(serde_json::json!({
            "apiVersion": "v1", "kind": "Namespace",
            "metadata": { "name": "payments", "deletionTimestamp": "2026-01-02T03:04:05Z" },
            "status": { "phase": "Terminating" },
        }))
        .unwrap(),
    ));
    let out = h
        .confirm_and_run(delete("a", "payments", &[]), ctx(Initiator::Ui))
        .unwrap();
    let Outcome::Completed(output) = out else {
        panic!("expected completion");
    };
    assert_eq!(output.data.unwrap()["outcome"], "deleting");
    assert!(output.message.unwrap().contains("terminating"));
}

#[test]
fn read_only_blocks_it_for_every_initiator() {
    let h = harness();
    h.connect("a", true);
    for initiator in [Initiator::Ui, Initiator::Command, Initiator::Agent] {
        let err = h
            .dispatch(delete("a", "payments", &[]), ctx(initiator))
            .unwrap_err();
        assert!(
            matches!(err, DispatchError::ReadOnly { .. }),
            "{initiator:?}"
        );
    }
    assert!(h.writes("a").is_empty());
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn system_namespaces_take_the_same_typed_name() {
    let h = harness();
    h.connect("a", false);
    h.allow_deletes("a", 4);
    for name in ["kube-system", "kube-public", "kube-node-lease", "default"] {
        let request = h.ask(delete("a", name, &[]), ctx(Initiator::Ui));
        assert_eq!(request.tier, ConfirmTier::TypeName, "{name}");
        assert_eq!(request.expected_name.as_deref(), Some(name));
    }
}

#[test]
fn it_refuses_a_target_that_is_not_a_namespace() {
    let h = harness();
    h.connect("a", false);
    let command = oxikube_domain::command::Command::NamespaceDelete {
        target: crate::testing::node("a", "worker-1"),
        contents: Default::default(),
    };
    // The handler refuses it in the guard's dry run, before anything is asked.
    let err = h.dispatch(command, ctx(Initiator::Ui)).unwrap_err();
    let DispatchError::Handler(err) = err else {
        panic!("a handler error");
    };
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(h.writes("a").is_empty());
    assert_eq!(h.bus.guard().pending_confirmations(), 0);
}

#[test]
fn a_missing_namespace_fails_the_rehearsal() {
    let h = harness();
    h.connect("a", false);
    h.resources("a")
        .script()
        .delete
        .push_err(oxikube_domain::OxiError::not_found(
            "namespaces \"gone\" not found",
        ));
    // The guard's dry run fails, so nobody is asked to type a name for a delete that cannot
    // happen.
    let err = h
        .dispatch(delete("a", "gone", &[]), ctx(Initiator::Ui))
        .unwrap_err();
    let DispatchError::Handler(err) = err else {
        panic!("a handler error");
    };
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert_eq!(
        deletes(&h),
        [(true, "gone".to_owned())],
        "the real delete never went out"
    );
    assert_eq!(h.bus.guard().pending_confirmations(), 0);
    let audit = h.audit();
    assert_eq!(audit.len(), 1);
    assert!(audit[0].dry_run && audit[0].outcome == AuditOutcome::Failed);
}

#[test]
fn a_server_that_cannot_dry_run_still_gets_the_confirmed_delete() {
    let h = harness();
    h.connect("a", false);
    h.allow_deletes("a", 1);
    let output =
        h.confirm_without_dry_runs("a", delete("a", "payments", &[]), ctx(Initiator::Agent));
    assert_eq!(
        output.message.as_deref(),
        Some("Deleted namespace payments")
    );
    assert_eq!(
        deletes(&h),
        [
            (true, "payments".to_owned()),
            (false, "payments".to_owned())
        ]
    );
}
