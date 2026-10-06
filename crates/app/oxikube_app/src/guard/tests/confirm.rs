//! The confirmation round trip through the bus: the tier comes from `CommandMeta`, the
//! first dispatch only asks, and the second needs the single-use token.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::safety::ConfirmTier;

use crate::command_bus::{DispatchError, Outcome};
use crate::guard::{Confirmation, ConfirmationError, MAX_PENDING_CONFIRMATIONS};
use crate::testing::{Harness, ctx, id, node, pod, pod_delete};

fn sample(id: CommandId) -> Command {
    match id {
        CommandId::NODE_DRAIN => Command::NodeDrain {
            target: node("a", "worker-1"),
            force: false,
        },
        CommandId::NODE_UNCORDON => Command::NodeUncordon {
            target: node("a", "worker-1"),
        },
        CommandId::POD_DELETE => pod_delete("a", "web-0"),
        CommandId::RESOURCE_DELETE => Command::ResourceDelete {
            target: pod("a", "web-0"),
            propagation: Default::default(),
        },
        CommandId::WORKLOAD_SCALE => Command::WorkloadScale {
            target: pod("a", "web-0"),
            replicas: 3,
        },
        other => panic!("no sample for {other}"),
    }
}

#[test]
fn first_dispatch_asks_with_the_tier_from_command_meta() {
    let h = Harness::new();
    h.connect("a", false);
    let expected = [
        (CommandId::NODE_UNCORDON, ConfirmTier::Simple), // Risk::Low
        (CommandId::POD_DELETE, ConfirmTier::Simple),    // Risk::Medium
        (CommandId::WORKLOAD_SCALE, ConfirmTier::Simple), // Risk::Medium
        (CommandId::RESOURCE_DELETE, ConfirmTier::Simple), // Risk::Medium, a Pod
        (CommandId::NODE_DRAIN, ConfirmTier::TypeName),  // Risk::High
    ];
    for (command, tier) in expected {
        let cmd = sample(command);
        let request = h.ask(cmd.clone(), ctx(Initiator::Ui));
        assert_eq!(request.command, command);
        assert_eq!(request.tier, tier, "{command}");
        assert_eq!(request.tier, cmd.meta().confirm, "{command}");
        assert_eq!(request.risk, cmd.meta().risk);
        assert_eq!(request.cluster, id("a"));
        assert!(request.summary.ends_with(" on a"), "{}", request.summary);
        let name = cmd.target().unwrap().name.to_string();
        match tier {
            ConfirmTier::TypeName => assert_eq!(request.expected_name, Some(name)),
            _ => assert_eq!(request.expected_name, None),
        }
    }
    assert!(h.calls().is_empty(), "nothing ran before confirmation");
    assert!(h.writes("a").is_empty());
    assert!(h.audit().is_empty(), "asking is not an audited attempt");
    assert_eq!(h.bus.guard().pending_confirmations(), 5);
}

#[test]
fn second_dispatch_with_the_token_runs_once() {
    let h = Harness::new();
    h.connect("a", false);
    h.allow_deletes("a", 2);
    let cmd = pod_delete("a", "web-0");

    let request = h.ask(cmd.clone(), ctx(Initiator::Command));
    let confirmed = ctx(Initiator::Command).with_confirmation(Confirmation::simple(request.token));
    let out = h.dispatch(cmd.clone(), confirmed.clone()).unwrap();
    assert!(matches!(out, Outcome::Completed(ref o) if o.message.as_deref() == Some("done")));
    assert_eq!(h.writes("a").len(), 1);
    let calls = h.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].mutation && !calls[0].dry_run);

    let reused = h.dispatch(cmd, confirmed).unwrap_err();
    assert!(
        matches!(
            reused,
            DispatchError::Confirmation(ConfirmationError::Unknown)
        ),
        "{reused:?}"
    );
    assert_eq!(h.writes("a").len(), 1, "a token is single-use");
    assert_eq!(
        h.outcomes(),
        [
            (AuditOutcome::Succeeded, Initiator::Command),
            (AuditOutcome::Denied, Initiator::Command)
        ]
    );
}

#[test]
fn a_token_is_bound_to_its_command_initiator_and_dry_run() {
    let h = Harness::new();
    h.connect("a", false);
    let mismatch = |answer_for: Command, ctx2: crate::DispatchContext| {
        let request = h.ask(pod_delete("a", "web-0"), ctx(Initiator::Ui));
        let err = h
            .dispatch(
                answer_for,
                ctx2.with_confirmation(Confirmation::simple(request.token)),
            )
            .unwrap_err();
        assert!(
            matches!(
                err,
                DispatchError::Confirmation(ConfirmationError::Mismatch)
            ),
            "{err:?}"
        );
    };
    mismatch(pod_delete("a", "web-1"), ctx(Initiator::Ui));
    mismatch(pod_delete("a", "web-0"), ctx(Initiator::Agent));
    mismatch(
        pod_delete("a", "web-0"),
        ctx(Initiator::Ui).with_dry_run(true),
    );
    assert!(h.writes("a").is_empty());
    assert_eq!(
        h.bus.guard().pending_confirmations(),
        0,
        "failed answers consume the token"
    );
}

#[test]
fn type_name_needs_the_exact_name() {
    let h = Harness::new();
    h.connect("a", false);
    h.allow_deletes("a", 1);
    let cmd = sample(CommandId::NODE_DRAIN);

    for wrong in [Confirmation::simple, |t| Confirmation::typed(t, "worker-2")] {
        let request = h.ask(cmd.clone(), ctx(Initiator::Ui));
        let err = h
            .dispatch(
                cmd.clone(),
                ctx(Initiator::Ui).with_confirmation(wrong(request.token)),
            )
            .unwrap_err();
        assert!(
            matches!(
                err,
                DispatchError::Confirmation(ConfirmationError::NameMismatch { ref expected })
                    if expected == "worker-1"
            ),
            "{err:?}"
        );
    }
    assert!(h.writes("a").is_empty());

    let out = h.confirm_and_run(cmd, ctx(Initiator::Ui)).unwrap();
    assert!(matches!(out, Outcome::Completed(_)));
    assert_eq!(h.writes("a").len(), 1);
}

#[test]
fn an_unknown_token_is_refused_and_audited() {
    let h = Harness::new();
    h.connect("a", false);
    let request = h.ask(pod_delete("a", "web-0"), ctx(Initiator::Ui));
    h.bus.decline(request.token).now_or_never_ok();
    let err = h
        .dispatch(
            pod_delete("a", "web-0"),
            ctx(Initiator::Ui).with_confirmation(Confirmation::simple(request.token)),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        DispatchError::Confirmation(ConfirmationError::Unknown)
    ));
    assert_eq!(
        h.outcomes(),
        [
            (AuditOutcome::Cancelled, Initiator::Ui),
            (AuditOutcome::Denied, Initiator::Ui)
        ]
    );
}

#[test]
fn declining_records_a_cancelled_attempt() {
    let h = Harness::new();
    h.connect("a", false);
    let request = h.ask(pod_delete("a", "web-0"), ctx(Initiator::Agent));
    h.bus.decline(request.token).now_or_never_ok();

    let audit = h.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Cancelled);
    assert_eq!(audit[0].initiator, Initiator::Agent);
    assert_eq!(audit[0].target, pod("a", "web-0"));
    assert_eq!(audit[0].cmd.as_ref(), "pod::Delete");
    assert!(h.writes("a").is_empty());

    let again = futures::FutureExt::now_or_never(h.bus.decline(request.token)).unwrap();
    assert!(matches!(
        again,
        Err(DispatchError::Confirmation(ConfirmationError::Unknown))
    ));
}

#[test]
fn pending_confirmations_are_bounded() {
    let h = Harness::new();
    h.connect("a", false);
    let first = h.ask(pod_delete("a", "web-0"), ctx(Initiator::Ui));
    for i in 0..MAX_PENDING_CONFIRMATIONS {
        h.ask(
            pod_delete("a", &format!("web-{}", i + 1)),
            ctx(Initiator::Ui),
        );
    }
    assert_eq!(
        h.bus.guard().pending_confirmations(),
        MAX_PENDING_CONFIRMATIONS
    );
    let err = h
        .dispatch(
            pod_delete("a", "web-0"),
            ctx(Initiator::Ui).with_confirmation(Confirmation::simple(first.token)),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        DispatchError::Confirmation(ConfirmationError::Unknown)
    ));
}

#[test]
fn dry_run_reaches_the_handler_and_the_record() {
    let h = Harness::new();
    h.connect("a", false);
    h.allow_deletes("a", 1);
    let out = h.confirm_and_run(
        pod_delete("a", "web-0"),
        ctx(Initiator::Ui).with_dry_run(true),
    );
    assert!(out.is_ok(), "{out:?}");
    let writes = h.writes("a");
    assert_eq!(writes.len(), 1);
    assert!(writes[0].is_dry_run());
    assert!(h.calls()[0].dry_run);
    let audit = h.audit();
    assert!(audit[0].dry_run);
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
}

trait NowOrNeverOk {
    fn now_or_never_ok(self);
}

impl<F: std::future::Future<Output = Result<(), DispatchError>>> NowOrNeverOk for F {
    fn now_or_never_ok(self) {
        futures::FutureExt::now_or_never(Box::pin(self))
            .expect("does not wait")
            .expect("ok");
    }
}
