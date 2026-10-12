//! Evict, delete and force delete one pod: the subresource and the delete parameters, the 429 from
//! a PodDisruptionBudget, the confirmation tiers, read-only mode and the audit trail.

use oxikube_domain::EvictionBlocked;
use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_ports::{DeleteOutcome, PropagationPolicy};
use oxikube_testkit::ResourceCall;

use super::*;
use crate::command_bus::DispatchError;

fn evict(name: &str) -> Command {
    Command::PodEvict {
        target: pod_ref(name),
    }
}

fn delete(name: &str, grace_period_seconds: Option<u32>) -> Command {
    Command::PodDelete {
        target: pod_ref(name),
        grace_period_seconds,
    }
}

fn force(name: &str) -> Command {
    Command::PodForceDelete {
        target: pod_ref(name),
    }
}

fn pod_commands() -> Vec<Command> {
    vec![
        evict("web-0"),
        delete("web-0", None),
        force("web-0"),
        Command::PodSanitize {
            cluster: id("a"),
            namespace: Some("default".into()),
            targets: vec![pod_ref("web-0")],
        },
    ]
}

fn with_pod() -> Workloads {
    let t = Workloads::new(false);
    t.port().insert(pod("web-0", "Running"));
    t
}

fn blocked() -> OxiError {
    OxiError::network("evicting pod default/web-0 is blocked by a PodDisruptionBudget")
        .with_retryable(true)
        .with_source(EvictionBlocked {
            reason: "The disruption budget web needs 2 healthy pods and has 2 currently".into(),
        })
}

#[test]
fn the_pod_commands_are_guarded_mutations_with_tool_stubs() {
    let tiers = [
        (CommandId::POD_EVICT, Risk::Medium, "k8s.pod_evict"),
        (CommandId::POD_DELETE, Risk::Medium, "k8s.pod_delete"),
        (
            CommandId::POD_FORCE_DELETE,
            Risk::High,
            "k8s.pod_force_delete",
        ),
        (CommandId::POD_SANITIZE, Risk::Medium, "k8s.pod_sanitize"),
    ];
    for (id, risk, tool) in tiers {
        let meta = command::lookup(id).expect("declared");
        assert!(meta.mutating, "{id}");
        assert_eq!(meta.risk, Some(risk), "{id}");
        assert!(meta.availability.requires_writable, "{id}");
        assert_eq!(id.tool_name(), tool);
    }
    assert_eq!(super::super::POD_COMMANDS.len(), 4);
}

#[test]
fn evict_and_delete_take_a_simple_confirm_and_force_delete_the_typed_name() {
    let t = with_pod();
    let cases = [
        (evict("web-0"), ConfirmTier::Simple, Risk::Medium),
        (delete("web-0", None), ConfirmTier::Simple, Risk::Medium),
        (delete("web-0", Some(30)), ConfirmTier::Simple, Risk::Medium),
        (force("web-0"), ConfirmTier::TypeName, Risk::High),
        // A delete spelled with grace 0 cannot dodge the typed name.
        (delete("web-0", Some(0)), ConfirmTier::TypeName, Risk::High),
    ];
    for (command, tier, risk) in cases {
        let request = t.h.ask(
            command.clone(),
            DispatchContext::new(Initiator::Ui, "alice"),
        );
        assert_eq!(request.tier, tier, "{command:?}");
        assert_eq!(request.risk, Some(risk));
        match tier {
            ConfirmTier::TypeName => assert_eq!(request.expected_name.as_deref(), Some("web-0")),
            _ => assert_eq!(request.expected_name, None),
        }
    }
    assert!(t.port().recorded_calls().is_empty(), "asking sends nothing");
}

#[test]
fn a_wrong_typed_name_does_not_force_delete() {
    use crate::guard::Confirmation;
    let t = with_pod();
    let ctx = || DispatchContext::new(Initiator::Ui, "alice");
    let request = t.h.ask(force("web-0"), ctx());
    let wrong = Confirmation::typed(request.token, "web-1");
    let error =
        t.h.dispatch(force("web-0"), ctx().with_confirmation(wrong))
            .unwrap_err();
    assert!(matches!(error, DispatchError::Confirmation(_)), "{error:?}");
    assert!(t.port().mutating_calls().is_empty());
}

#[test]
fn evict_rehearses_then_calls_the_eviction_subresource() {
    let t = with_pod();
    t.port().script().evict.push_ok(()).push_ok(());
    let out = t.run(evict("web-0"), Initiator::Ui).expect("evict");
    assert_eq!(out.message.as_deref(), Some("Evicted Pod default/web-0"));

    let calls = t.port().mutating_calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    let ResourceCall::Evict {
        namespace,
        pod,
        options,
    } = &calls[0]
    else {
        panic!("the first write is an eviction: {calls:?}");
    };
    assert_eq!((namespace.as_str(), pod.as_str()), ("default", "web-0"));
    assert!(options.dry_run, "the server rehearses first");
    assert!(matches!(&calls[1], ResourceCall::Evict { options, .. } if !options.dry_run));
    let audit = t.h.audit();
    assert_eq!((audit.len(), audit[0].cmd.as_ref()), (1, "pod::Evict"));
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
}

#[test]
fn a_budget_refusal_says_which_budget_and_evicts_nothing() {
    let t = with_pod();
    t.port().script().evict.push_err(blocked());
    let error = t.run(evict("web-0"), Initiator::Ui).unwrap_err();
    let DispatchError::Handler(error) = error else {
        panic!("a handler error: {error:?}");
    };
    let text = error.message();
    assert!(
        text.contains("blocked by disruption budget web"),
        "names the budget: {text}"
    );
    assert!(
        !text.contains("429") && !text.contains("TooManyRequests"),
        "{text}"
    );
    assert!(error.is_retryable(), "the user can try again later");
    // The rehearsal was refused: no real eviction followed.
    assert_eq!(t.port().mutating_calls().len(), 1);
    assert_eq!(t.h.audit()[0].outcome, AuditOutcome::Failed);
}

#[test]
fn an_eviction_dispatched_as_a_dry_run_stops_after_the_rehearsal() {
    let t = with_pod();
    t.port().script().evict.push_ok(());
    let out = t.dry_run(evict("web-0")).expect("dry run");
    assert_eq!(
        out.message.as_deref(),
        Some("Dry run: Pod default/web-0 can be evicted")
    );
    assert_eq!(t.port().mutating_calls().len(), 1);
    assert!(t.h.audit()[0].dry_run);
}

#[test]
fn delete_uses_the_pods_own_grace_period_and_force_delete_uses_zero() {
    let t = with_pod();
    t.h.allow_deletes("a", 6);
    t.run(delete("web-0", None), Initiator::Ui).expect("delete");
    t.run(delete("web-0", Some(10)), Initiator::Ui)
        .expect("delete");
    let out = t.run(force("web-0"), Initiator::Ui).expect("force delete");
    assert_eq!(
        out.message.as_deref(),
        Some("Force deleted Pod default/web-0")
    );

    let deletes: Vec<_> = t
        .port()
        .mutating_calls()
        .into_iter()
        .map(|call| match call {
            ResourceCall::Delete { name, options, .. } => {
                assert_eq!(name, "web-0");
                options
            }
            other => panic!("a delete: {other:?}"),
        })
        .collect();
    // Each: the server's rehearsal, then the delete.
    let grace: Vec<_> = deletes
        .iter()
        .map(|o| (o.dry_run, o.grace_period_secs))
        .collect();
    assert_eq!(
        grace,
        [
            (true, None),
            (false, None),
            (true, Some(10)),
            (false, Some(10)),
            (true, Some(0)),
            (false, Some(0)),
        ]
    );
    assert!(
        deletes
            .iter()
            .all(|o| o.propagation == Some(PropagationPolicy::Background))
    );
    let audit = t.h.audit();
    assert_eq!(
        audit.iter().map(|r| r.cmd.as_ref()).collect::<Vec<_>>(),
        ["pod::Delete", "pod::Delete", "pod::ForceDelete"]
    );
    assert_eq!(audit[0].detail, None);
    assert_eq!(audit[1].detail.as_deref(), Some("grace=10"));
    assert_eq!(audit[2].detail.as_deref(), Some("grace=0"));
}

#[test]
fn a_pod_still_shutting_down_reads_as_deleting() {
    let t = with_pod();
    t.port().script().delete.push_ok(DeleteOutcome::Deleted);
    t.port()
        .script()
        .delete
        .push_ok(DeleteOutcome::Deleting(pod("web-0", "Running")));
    let out = t.run(delete("web-0", None), Initiator::Ui).expect("delete");
    assert_eq!(out.message.as_deref(), Some("Deleting Pod default/web-0"));
    assert_eq!(out.data.unwrap()["outcome"], "deleting");
}

#[test]
fn a_delete_dispatched_as_a_dry_run_stops_after_the_rehearsal() {
    let t = with_pod();
    let out = t.dry_run(force("web-0")).expect("dry run");
    assert_eq!(
        out.message.as_deref(),
        Some("Dry run: Pod default/web-0 can be force deleted")
    );
    assert_eq!(t.port().mutating_calls().len(), 1);
    assert_eq!(
        t.objects_of(&Gvk::new("", "v1", "Pod")).len(),
        1,
        "still there"
    );
}

#[test]
fn something_that_is_not_a_pod_is_refused() {
    let t = with_pod();
    let target = cronjob_ref("nightly");
    let error = t
        .run(Command::PodEvict { target }, Initiator::Ui)
        .unwrap_err();
    assert!(matches!(error, DispatchError::Handler(_)), "{error:?}");
    assert!(t.port().mutating_calls().is_empty());
}

#[test]
fn a_read_only_cluster_refuses_every_pod_command_before_any_request() {
    let t = Workloads::new(true);
    for initiator in [
        Initiator::Ui,
        Initiator::Command,
        Initiator::Agent,
        Initiator::Plugin,
    ] {
        for command in pod_commands() {
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
    assert_eq!(audit.len(), 16);
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn every_pod_command_carries_its_initiator_into_the_audit_record() {
    let t = with_pod();
    t.port().script().evict.push_ok(()).push_ok(());
    t.run(evict("web-0"), Initiator::Agent).expect("evict");
    let record = &t.h.audit()[0];
    assert_eq!(record.initiator, Initiator::Agent);
    assert_eq!(&*record.who, "alice");
    assert_eq!(&*record.target.name, "web-0");
}
