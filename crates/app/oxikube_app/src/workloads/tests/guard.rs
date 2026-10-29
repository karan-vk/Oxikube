//! The guard around the workload commands: read-only mode, the confirmation tier, the audit trail
//! and the tool stubs.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};

use super::*;
use crate::command_bus::DispatchError;

fn every_command() -> Vec<Command> {
    vec![
        trigger("nightly"),
        Command::CronJobSuspend {
            target: cronjob_ref("nightly"),
        },
        Command::CronJobResume {
            target: cronjob_ref("nightly"),
        },
        Command::JobSuspend {
            target: job_ref("import"),
        },
        Command::JobResume {
            target: job_ref("import"),
        },
    ]
}

#[test]
fn every_command_is_a_low_risk_mutation_with_a_simple_confirm_and_a_tool_stub() {
    for command in every_command() {
        let meta = command::lookup(command.id()).expect("declared");
        assert!(meta.mutating, "{}", command.id());
        assert_eq!(meta.risk, Some(Risk::Low), "{}", command.id());
        assert_eq!(meta.confirm, ConfirmTier::Simple, "{}", command.id());
        assert!(meta.availability.requires_writable);
        assert!(command.id().tool_name().starts_with("k8s."));
    }
    let tools: Vec<_> = super::super::WORKLOAD_COMMANDS
        .iter()
        .map(|id| id.tool_name())
        .collect();
    assert_eq!(
        tools,
        [
            "k8s.cronjob_trigger",
            "k8s.cronjob_suspend",
            "k8s.cronjob_resume",
            "k8s.job_suspend",
            "k8s.job_resume"
        ]
    );
}

#[test]
fn every_command_asks_before_it_does_anything() {
    let t = Workloads::new(false);
    t.port()
        .insert(job("import", json!({"suspend": true}), json!({})));
    for command in every_command() {
        let ctx = DispatchContext::new(Initiator::Ui, "alice");
        let request = t.h.ask(command.clone(), ctx);
        assert_eq!(request.tier, ConfirmTier::Simple, "{}", command.id());
        assert_eq!(request.risk, Some(Risk::Low));
    }
    // The guard's dry runs (E19-S07) are all that is sent before the answer.
    assert!(
        t.port()
            .mutating_calls()
            .iter()
            .all(oxikube_testkit::ResourceCall::is_dry_run),
        "no request before the answer"
    );
}

#[test]
fn a_read_only_cluster_refuses_every_command_for_every_initiator() {
    let t = Workloads::new(true);
    for initiator in [
        Initiator::Ui,
        Initiator::Command,
        Initiator::Agent,
        Initiator::Plugin,
    ] {
        for command in every_command() {
            // Refused at the door: before any question, for a person and an agent alike.
            let ctx = DispatchContext::new(initiator, "alice");
            let error = t.h.dispatch(command.clone(), ctx).unwrap_err();
            assert!(
                matches!(error, DispatchError::ReadOnly { .. }),
                "{initiator:?} {}: {error:?}",
                command.id()
            );
        }
    }
    assert!(t.port().recorded_calls().is_empty(), "not even a read");
    let audit = t.h.audit();
    assert_eq!(audit.len(), 20);
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn a_trigger_is_audited_with_the_initiator_and_the_job_it_created() {
    let t = Workloads::new(false);
    t.run(trigger("nightly"), Initiator::Agent)
        .expect("trigger");
    let audit = t.h.audit();
    assert_eq!(audit.len(), 2, "the guard's dry run, then the trigger");
    assert!(audit[0].dry_run);
    let record = &audit[1];
    assert_eq!(record.cmd.as_ref(), "cronjob::Trigger");
    assert_eq!(record.initiator, Initiator::Agent);
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(&*record.target.name, "nightly");
    assert_eq!(record.detail.as_deref(), Some("job=nightly-manual-x7k2p"));
    assert!(!record.dry_run);
}

#[test]
fn a_failed_trigger_is_audited_as_failed_without_a_job() {
    let t = Workloads::new(false);
    let error = t.run(trigger("ghost"), Initiator::Ui).unwrap_err();
    assert!(matches!(error, DispatchError::Handler(_)), "{error:?}");
    let audit = t.h.audit();
    assert_eq!(audit[0].outcome, AuditOutcome::Failed);
    assert_eq!(audit[0].detail, None);
}

#[test]
fn a_dry_run_is_audited_as_one() {
    let t = Workloads::new(false);
    t.dry_run(trigger("nightly")).expect("dry run");
    let audit = t.h.audit();
    assert!(audit[0].dry_run);
    assert_eq!(audit[0].detail.as_deref(), Some("job=nightly-manual-x7k2p"));
}

#[test]
fn a_suspend_is_audited_against_the_object_with_no_body() {
    let t = Workloads::new(false);
    t.run(
        Command::CronJobSuspend {
            target: cronjob_ref("nightly"),
        },
        Initiator::Command,
    )
    .expect("suspend");
    let record = &t.h.audit()[1];
    assert!(!record.dry_run);
    assert_eq!(record.cmd.as_ref(), "cronjob::Suspend");
    assert_eq!(record.initiator, Initiator::Command);
    assert_eq!(record.detail, None);
    assert!(!format!("{record:?}").contains("suspend\":"), "{record:?}");
}

#[test]
fn a_declined_confirmation_sends_nothing_and_is_audited() {
    let t = Workloads::new(false);
    let ctx = DispatchContext::new(Initiator::Ui, "alice");
    let request = t.h.ask(trigger("nightly"), ctx);
    futures::FutureExt::now_or_never(t.h.bus.decline(request.token))
        .expect("answers")
        .expect("declined");
    assert!(
        t.port()
            .mutating_calls()
            .iter()
            .all(oxikube_testkit::ResourceCall::is_dry_run),
        "only the guard's dry run"
    );
    let audit = t.h.audit();
    assert!(audit[0].dry_run);
    assert_eq!(audit[1].outcome, AuditOutcome::Cancelled);
}

#[test]
fn the_handlers_register_once() {
    // Registering the same commands twice is a wiring bug the registry reports.
    let mut registry = crate::command_bus::CommandRegistry::new();
    let h = Workloads::new(false);
    super::super::register_commands(&mut registry, h.h.manager.clone()).expect("first");
    let again = super::super::register_commands(&mut registry, h.h.manager);
    assert!(matches!(
        again,
        Err(crate::command_bus::RegisterError::Duplicate { .. })
    ));
    assert!(registry.contains(CommandId::CRONJOB_TRIGGER));
}
