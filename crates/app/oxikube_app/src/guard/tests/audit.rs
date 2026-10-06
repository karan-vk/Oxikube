//! Audit records for allowed, denied and failed mutations, with the initiator; an audit
//! write failure fails the mutation closed.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::{ErrorKind, OxiError};

use crate::command_bus::{DispatchContext, DispatchError, Outcome};
use crate::testing::{Harness, ctx, id, pod, pod_delete};

/// The initiators that may run a non-privileged mutation.
const INITIATORS: [Initiator; 4] = Initiator::ALL;

#[test]
fn allowed_mutation_is_audited_as_succeeded_with_its_initiator() {
    for initiator in INITIATORS {
        let h = Harness::new();
        h.connect("a", false);
        h.allow_deletes("a", 1);
        let out = h.confirm_and_run(pod_delete("a", "web-0"), ctx(initiator));
        assert!(matches!(out, Ok(Outcome::Completed(_))), "{out:?}");

        let audit = h.audit();
        assert_eq!(audit.len(), 1, "{initiator}");
        let record = &audit[0];
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
        assert_eq!(record.initiator, initiator);
        assert_eq!(record.who.as_ref(), "alice");
        assert_eq!(record.cmd.as_ref(), "pod::Delete");
        assert_eq!(record.cluster, id("a"));
        assert_eq!(record.target, pod("a", "web-0"));
        assert!(!record.dry_run);
        assert_eq!(h.calls()[0].initiator, initiator);
    }
}

#[test]
fn denied_mutation_is_audited_as_denied_with_its_initiator() {
    for initiator in INITIATORS {
        let h = Harness::new();
        h.connect("a", true);
        assert!(
            h.dispatch(pod_delete("a", "web-0"), ctx(initiator))
                .is_err()
        );
        let audit = h.audit();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].outcome, AuditOutcome::Denied);
        assert_eq!(audit[0].initiator, initiator);
        assert_eq!(audit[0].target, pod("a", "web-0"));
    }
}

#[test]
fn failed_mutation_is_audited_as_failed_with_its_initiator() {
    for initiator in INITIATORS {
        let h = Harness::new();
        h.connect("a", false);
        h.resources("a")
            .script()
            .delete
            .push_err(OxiError::forbidden("pods \"web-0\" is forbidden"));
        let err = h
            .confirm_and_run(pod_delete("a", "web-0"), ctx(initiator))
            .unwrap_err();
        assert!(matches!(err, DispatchError::Handler(ref e) if e.kind() == ErrorKind::Forbidden));
        assert_eq!(
            OxiError::from(err).kind(),
            ErrorKind::Forbidden,
            "the handler's error passes through"
        );
        assert_eq!(h.writes("a").len(), 1, "the request reached the cluster");
        assert_eq!(h.outcomes(), [(AuditOutcome::Failed, initiator)]);
    }
}

#[test]
fn reads_are_not_audited() {
    let h = Harness::new();
    h.connect("a", false);
    h.dispatch(crate::testing::view_logs("a", "web-0"), ctx(Initiator::Ui))
        .unwrap();
    assert!(h.audit().is_empty());
}

#[test]
fn audit_write_failure_fails_the_mutation_closed() {
    let h = Harness::new();
    h.connect("a", false);
    h.allow_deletes("a", 2);
    let cmd = pod_delete("a", "web-0");

    // The record of the first mutation cannot be written: the caller sees a failure.
    let request = h.ask(cmd.clone(), ctx(Initiator::Ui));
    h.state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
    let err = h
        .dispatch(
            cmd.clone(),
            ctx(Initiator::Ui).with_confirmation(crate::Confirmation::simple(request.token)),
        )
        .unwrap_err();
    assert!(matches!(err, DispatchError::AuditFailed(_)), "{err:?}");
    assert_eq!(OxiError::from(err).kind(), ErrorKind::Internal);
    assert!(h.audit().is_empty());
    assert_eq!(h.bus.guard().audit().backlog_len(), 1);
    let writes_after_first = h.writes("a").len();

    // While the log stays unwritable, the next mutation is refused before any request.
    h.state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
    let err = h
        .confirm_and_run(cmd.clone(), ctx(Initiator::Ui))
        .unwrap_err();
    assert!(matches!(err, DispatchError::AuditUnavailable(_)), "{err:?}");
    assert_eq!(
        h.writes("a").len(),
        writes_after_first,
        "no request while the log is down"
    );

    // Once the store recovers, the backlog lands first and the mutation runs.
    let out = h.confirm_and_run(cmd, ctx(Initiator::Ui));
    assert!(matches!(out, Ok(Outcome::Completed(_))), "{out:?}");
    assert_eq!(h.bus.guard().audit().backlog_len(), 0);
    assert_eq!(
        h.outcomes(),
        [
            (AuditOutcome::Succeeded, Initiator::Ui),
            (AuditOutcome::Succeeded, Initiator::Ui)
        ]
    );
}

#[test]
fn a_refusal_stands_when_its_record_cannot_be_written() {
    let h = Harness::new();
    h.connect("a", true);
    h.state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
    let err = h
        .dispatch(pod_delete("a", "web-0"), ctx(Initiator::Agent))
        .unwrap_err();
    assert!(matches!(err, DispatchError::ReadOnly { .. }), "{err:?}");
    assert_eq!(h.bus.guard().audit().backlog_len(), 1);
}

#[test]
fn who_is_redacted_and_no_payload_is_recorded() {
    let h = Harness::new();
    h.connect("a", true);
    let who = "agent Authorization: Bearer abc.def.ghi";
    let ctx = DispatchContext::new(Initiator::Agent, who);
    assert!(h.dispatch(pod_delete("a", "web-0"), ctx).is_err());
    let record = &h.audit()[0];
    assert!(!record.who.contains("abc.def.ghi"), "{}", record.who);
    assert!(record.who.contains("[redacted]"), "{}", record.who);
}
