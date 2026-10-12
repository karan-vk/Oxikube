//! `workload::Rollback` through the guard: tier, read-only, audit with from and to.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{self, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};

use super::{Rollouts, deployment_object, deployment_ref, replicaset_ref, rollback};
use crate::command_bus::DispatchError;
use crate::testing::ctx;

#[test]
fn it_is_a_guarded_medium_risk_command_with_a_tool_stub() {
    let meta = command::lookup(CommandId::WORKLOAD_ROLLBACK).unwrap();
    assert!(meta.mutating);
    assert_eq!(meta.risk, Some(Risk::Medium));
    assert_eq!(meta.id.tool_name(), "k8s.workload_rollback");
    assert_eq!(meta.title, "Rollback Workload");
}

#[test]
fn it_asks_once_at_medium_from_every_door_and_sends_nothing_before_the_answer() {
    let t = Rollouts::new(false);
    for initiator in Initiator::ALL {
        let request = t.h.ask(rollback(deployment_ref(), Some(2)), ctx(initiator));
        assert_eq!(request.risk, Some(Risk::Medium), "{initiator}");
        assert_eq!(request.tier, ConfirmTier::Simple, "{initiator}");
        assert!(
            request
                .summary
                .starts_with("Roll back Deployment shop/web to revision 2 on "),
            "{}",
            request.summary
        );
    }
    let previous =
        t.h.ask(rollback(deployment_ref(), None), ctx(Initiator::Ui));
    assert!(
        previous.summary.contains("to the previous revision"),
        "{}",
        previous.summary
    );
    assert!(
        t.port().recorded_calls().is_empty(),
        "not even a read before the answer"
    );
}

#[test]
fn a_read_only_cluster_refuses_every_initiator_before_any_request() {
    let t = Rollouts::new(true);
    for initiator in Initiator::ALL {
        let error =
            t.h.dispatch(rollback(deployment_ref(), None), ctx(initiator))
                .unwrap_err();
        assert!(
            matches!(error, DispatchError::ReadOnly { .. }),
            "{initiator}"
        );
    }
    assert!(
        t.port().recorded_calls().is_empty(),
        "no request of any kind"
    );
    assert!(
        t.h.audit()
            .iter()
            .all(|r| r.outcome == AuditOutcome::Denied)
    );
    assert_eq!(t.h.audit().len(), Initiator::ALL.len());
}

#[test]
fn the_audit_record_names_the_workload_the_initiator_and_the_revisions_without_secrets() {
    let t = Rollouts::new(false);
    t.answer_patches();
    t.h.confirm_and_run(rollback(deployment_ref(), Some(1)), ctx(Initiator::Ui))
        .unwrap();
    let audit = t.h.audit();
    assert_eq!(audit.len(), 1);
    let record = &audit[0];
    assert_eq!(&*record.cmd, "workload::Rollback");
    assert_eq!(record.target, deployment_ref());
    assert_eq!(record.initiator, Initiator::Ui);
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(record.detail.as_deref(), Some("to_revision=1 from=3 to=1"));
}

#[test]
fn a_refused_rollback_is_audited_as_failed_and_the_default_revision_is_named() {
    let t = Rollouts::with(false, deployment_object(3, "web:v3", true));
    t.h.confirm_and_run(rollback(deployment_ref(), None), ctx(Initiator::Agent))
        .unwrap_err();
    let record = &t.h.audit()[0];
    assert_eq!(record.outcome, AuditOutcome::Failed);
    assert_eq!(record.initiator, Initiator::Agent);
    assert_eq!(record.detail.as_deref(), Some("to_revision=previous"));
}

#[test]
fn declining_rolls_nothing_back_and_is_audited_as_cancelled() {
    let t = Rollouts::new(false);
    let request =
        t.h.ask(rollback(deployment_ref(), Some(2)), ctx(Initiator::Ui));
    futures::executor::block_on(t.h.bus.decline(request.token)).unwrap();
    assert!(t.port().mutating_calls().is_empty());
    assert_eq!(t.h.audit()[0].outcome, AuditOutcome::Cancelled);
}

#[test]
fn a_replicasets_confirmation_says_its_deployment_is_what_goes_back() {
    let t = Rollouts::new(false);
    let request =
        t.h.ask(rollback(replicaset_ref("web-1"), None), ctx(Initiator::Ui));
    assert!(
        request
            .summary
            .starts_with("Roll back the Deployment that owns ReplicaSet shop/web-1 to this ReplicaSet's revision on "),
        "{}",
        request.summary
    );
}
