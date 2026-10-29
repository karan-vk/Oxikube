//! The mandatory dry run (E19-S02): an `Irreversible` command is dry-run by the guard before the
//! confirmation is asked; supported, unsupported and refused dry runs; a rehearsal never writes.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId, Propagation};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{DeleteOptions, DeleteOutcome};
use oxikube_testkit::ResourceCall;

use crate::command_bus::{CommandOutput, DispatchError, HandlerContext, Outcome};
use crate::guard::{Confirmation, DryRunPreview};
use crate::testing::{Harness, ctx, declared, id, pod};

fn namespace_delete(name: &str) -> Command {
    Command::ResourceDelete {
        target: ResourceRef::cluster_scoped(id("a"), Gvk::new("", "v1", "Namespace"), name),
        propagation: Propagation::Background,
    }
}

/// `(dry_run, name)` of every delete `a` received, in order.
fn deletes(h: &Harness) -> Vec<(bool, String)> {
    h.writes("a")
        .into_iter()
        .map(|call| match call {
            ResourceCall::Delete { options, name, .. } => (options.dry_run, name),
            other => panic!("unexpected write {other:?}"),
        })
        .collect()
}

fn answer(request: &crate::ConfirmationRequest) -> Confirmation {
    Confirmation::typed(
        request.token,
        request.expected_name.clone().expect("a typed name"),
    )
}

#[test]
fn an_irreversible_command_is_dry_run_before_it_is_asked_then_runs_once_confirmed() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    h.allow_deletes("a", 3);
    let command = namespace_delete("payments");

    let request = h.ask(command.clone(), ctx(Initiator::Ui));
    assert_eq!(request.initiator, Initiator::Ui);
    let Some(DryRunPreview::Rehearsed(output)) = &request.preview else {
        panic!("expected a dry run before the question: {request:?}");
    };
    assert!(
        request
            .preview
            .as_ref()
            .is_some_and(DryRunPreview::is_available)
    );
    let message = output.message.as_deref().unwrap_or_default();
    assert!(
        message.starts_with("Dry run:") && message.contains("payments"),
        "{message}"
    );
    assert_eq!(deletes(&h), [(true, "payments".to_owned())]);
    let audit = h.audit();
    assert_eq!(audit.len(), 1, "the dry run is an audited attempt");
    assert!(audit[0].dry_run);
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);

    let out = h
        .dispatch(
            command,
            ctx(Initiator::Ui).with_confirmation(answer(&request)),
        )
        .unwrap();
    assert!(matches!(out, Outcome::Completed(_)), "{out:?}");
    assert_eq!(
        deletes(&h),
        [
            (true, "payments".to_owned()),
            (true, "payments".to_owned()),
            (false, "payments".to_owned()),
        ],
        "the guard's dry run, the handler's own right before the delete, the delete"
    );
    assert_eq!(
        h.outcomes(),
        [
            (AuditOutcome::Succeeded, Initiator::Ui),
            (AuditOutcome::Succeeded, Initiator::Ui)
        ]
    );
}

#[test]
fn an_unsupported_dry_run_is_marked_no_preview_and_never_blocks() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    let resources = h.resources("a");
    resources.script().delete.push_err(OxiError::new(
        ErrorKind::Unsupported,
        "the aggregated API does not support dry run",
    ));
    resources.script().delete.push_ok(DeleteOutcome::Deleted);
    resources.script().delete.push_ok(DeleteOutcome::Deleted);
    let command = namespace_delete("metrics");

    let request = h.ask(command.clone(), ctx(Initiator::Agent));
    assert_eq!(
        request.preview,
        Some(DryRunPreview::NoPreviewAvailable {
            reason: "the aggregated API does not support dry run".to_owned()
        })
    );
    let out = h.dispatch(
        command,
        ctx(Initiator::Agent).with_confirmation(answer(&request)),
    );
    assert!(matches!(out, Ok(Outcome::Completed(_))), "{out:?}");
    assert_eq!(deletes(&h).last(), Some(&(false, "metrics".to_owned())));
    assert_eq!(
        h.outcomes(),
        [
            (AuditOutcome::Failed, Initiator::Agent),
            (AuditOutcome::Succeeded, Initiator::Agent)
        ],
        "the dry run that could not run, then the delete"
    );
}

#[test]
fn a_refused_dry_run_refuses_the_command_and_asks_nothing() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    h.resources("a").script().delete.push_err(OxiError::new(
        ErrorKind::Forbidden,
        "namespaces is forbidden",
    ));

    let err = h
        .dispatch(namespace_delete("payments"), ctx(Initiator::Command))
        .unwrap_err();
    let DispatchError::Handler(error) = &err else {
        panic!("the server's error: {err:?}");
    };
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert_eq!(h.bus.guard().pending_confirmations(), 0, "nothing asked");
    assert_eq!(deletes(&h), [(true, "payments".to_owned())]);
    let audit = h.audit();
    assert_eq!(audit.len(), 1);
    assert!(audit[0].dry_run);
    assert_eq!(audit[0].outcome, AuditOutcome::Failed);
}

#[test]
fn a_rehearsal_sends_every_request_as_a_dry_run_even_from_a_careless_handler() {
    // A `resource::Delete` handler that ignores `Mutation::delete_options` and always asks for a
    // real delete.
    let h = Harness::build(&[CommandId::RESOURCE_DELETE], |reg, _, _| {
        reg.register(
            declared(CommandId::RESOURCE_DELETE),
            |command: Command, cx: HandlerContext| async move {
                let mutation = cx.require_mutation()?;
                let target = command.target().expect("a target").clone();
                mutation
                    .writer()
                    .delete(
                        &target.gvk,
                        target.namespace(),
                        &target.name,
                        &DeleteOptions::default(),
                    )
                    .await?;
                Ok(CommandOutput::message("deleted"))
            },
        )
    });
    h.connect("a", false);
    h.allow_deletes("a", 1);
    let request = h.ask(namespace_delete("payments"), ctx(Initiator::Plugin));
    assert!(request.preview.is_some());
    assert_eq!(
        deletes(&h),
        [(true, "payments".to_owned())],
        "the gate forced the dry run"
    );
}

#[test]
fn a_dry_run_dispatch_and_ordinary_commands_are_not_rehearsed() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    // A dispatch that is a dry run already needs no second one.
    let request = h.ask(
        namespace_delete("payments"),
        ctx(Initiator::Ui).with_dry_run(true),
    );
    assert_eq!(request.preview, None);
    // A medium-risk delete is asked straight away.
    let request = h.ask(
        Command::ResourceDelete {
            target: pod("a", "web-0"),
            propagation: Propagation::Background,
        },
        ctx(Initiator::Ui),
    );
    assert_eq!(request.preview, None);
    assert!(h.writes("a").is_empty());
    assert!(h.audit().is_empty(), "asking alone is not an attempt");
}

#[test]
fn the_dry_run_needs_a_writable_audit_log() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    h.allow_deletes("a", 1);
    h.state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
    // An earlier refusal is stuck in the backlog: the dry run is refused before any request.
    h.connect("b", true);
    let _ = h.dispatch(crate::testing::pod_delete("b", "web-0"), ctx(Initiator::Ui));
    h.state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
    let err = h
        .dispatch(namespace_delete("payments"), ctx(Initiator::Ui))
        .unwrap_err();
    assert!(matches!(err, DispatchError::AuditUnavailable(_)), "{err:?}");
    assert!(h.writes("a").is_empty());
    assert_eq!(h.bus.guard().pending_confirmations(), 0);
}

#[test]
fn a_confirmation_without_the_dry_run_cannot_approve_an_irreversible_command() {
    use crate::command_bus::DispatchContext;
    use crate::guard::rehearsal::require_rehearsal;

    let command = namespace_delete("payments");
    let meta = command.meta();
    let real = DispatchContext::new(Initiator::Ui, "alice");
    assert!(matches!(
        require_rehearsal(meta, &command, &real, false),
        Err(DispatchError::DryRunRequired(CommandId::RESOURCE_DELETE))
    ));
    assert!(require_rehearsal(meta, &command, &real, true).is_ok());
    // A dry-run dispatch and a command that is not irreversible need none.
    assert!(require_rehearsal(meta, &command, &real.clone().with_dry_run(true), false).is_ok());
    let ordinary = Command::ResourceDelete {
        target: pod("a", "web-0"),
        propagation: Propagation::Background,
    };
    assert!(require_rehearsal(ordinary.meta(), &ordinary, &real, false).is_ok());
    assert_eq!(
        oxikube_domain::OxiError::from(DispatchError::DryRunRequired(CommandId::RESOURCE_DELETE))
            .kind(),
        ErrorKind::Validation
    );
}
