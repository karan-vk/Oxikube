//! [`CommandBus::execute`](crate::CommandBus::execute) over a [`ConfirmationProvider`]: the
//! confirmation round trip in one call, routed by initiator; declines and dropped waits are
//! audited as cancelled by the user. Also the reason every refusal records.

use std::sync::Arc;

use async_trait::async_trait;
use futures::FutureExt;
use oxikube_domain::audit::{AuditDenial, AuditOutcome, Initiator};
use oxikube_domain::safety::ConfirmTier;
use parking_lot::Mutex;

use crate::command_bus::{DispatchError, Outcome};
use crate::guard::{ConfirmationProvider, ConfirmationRequest, ConfirmationRouter, Decision};
use crate::testing::{Harness, ctx, node, pod_delete};

/// Answers every request with `decision` and remembers what it was asked.
struct Scripted {
    decision: Decision,
    asked: Mutex<Vec<(Initiator, ConfirmTier)>>,
}

impl Scripted {
    fn new(decision: Decision) -> Arc<Self> {
        Arc::new(Self {
            decision,
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<(Initiator, ConfirmTier)> {
        self.asked.lock().clone()
    }
}

#[async_trait]
impl ConfirmationProvider for Scripted {
    async fn confirm(&self, request: &ConfirmationRequest) -> Decision {
        self.asked.lock().push((request.initiator, request.tier));
        self.decision.clone()
    }
}

/// Types the name the request asks for.
struct TypesTheName;

#[async_trait]
impl ConfirmationProvider for TypesTheName {
    async fn confirm(&self, request: &ConfirmationRequest) -> Decision {
        match &request.expected_name {
            Some(name) => Decision::typed(name.clone()),
            None => Decision::confirm(),
        }
    }
}

/// Never answers.
struct Silent;

#[async_trait]
impl ConfirmationProvider for Silent {
    async fn confirm(&self, _request: &ConfirmationRequest) -> Decision {
        futures::future::pending().await
    }
}

fn execute(
    h: &Harness,
    command: oxikube_domain::command::Command,
    initiator: Initiator,
    provider: &dyn ConfirmationProvider,
) -> Result<Outcome, DispatchError> {
    h.bus
        .execute(command, ctx(initiator), provider)
        .now_or_never()
        .expect("execute does not wait on fakes")
}

#[test]
fn execute_asks_the_provider_then_runs_the_command_once() {
    let h = Harness::new();
    h.connect("a", false);
    h.allow_deletes("a", 1);
    let provider = Scripted::new(Decision::confirm());

    let out = execute(
        &h,
        pod_delete("a", "web-0"),
        Initiator::Ui,
        provider.as_ref(),
    );
    assert!(matches!(out, Ok(Outcome::Completed(_))), "{out:?}");
    assert_eq!(provider.asked(), [(Initiator::Ui, ConfirmTier::Simple)]);
    assert_eq!(h.writes("a").len(), 1);
    assert_eq!(h.outcomes(), [(AuditOutcome::Succeeded, Initiator::Ui)]);
    assert_eq!(h.bus.guard().pending_confirmations(), 0);
}

#[test]
fn a_typed_name_goes_through_and_a_wrong_one_is_refused() {
    let h = Harness::new();
    h.connect("a", false);
    h.allow_deletes("a", 1);
    let drain = || oxikube_domain::command::Command::NodeDrain {
        target: node("a", "worker-1"),
        flags: oxikube_domain::drain::DrainFlags::default(),
    };

    let wrong = execute(
        &h,
        drain(),
        Initiator::Agent,
        Scripted::new(Decision::typed("worker-2")).as_ref(),
    );
    assert!(
        matches!(wrong, Err(DispatchError::Confirmation(_))),
        "{wrong:?}"
    );
    assert!(h.writes("a").is_empty());

    let out = execute(&h, drain(), Initiator::Agent, &TypesTheName);
    assert!(matches!(out, Ok(Outcome::Completed(_))), "{out:?}");
    assert_eq!(h.writes("a").len(), 1);
}

#[test]
fn a_decline_runs_nothing_and_is_audited_as_cancelled_by_the_user() {
    let h = Harness::new();
    h.connect("a", false);
    let err = execute(
        &h,
        pod_delete("a", "web-0"),
        Initiator::Agent,
        Scripted::new(Decision::Decline).as_ref(),
    )
    .unwrap_err();
    assert!(matches!(err, DispatchError::Declined(_)), "{err:?}");
    assert_eq!(err.audit_denial(), Some(AuditDenial::UserCancelled));
    assert!(h.calls().is_empty(), "no handler ran");
    assert!(h.writes("a").is_empty(), "no port call");
    let audit = h.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Cancelled);
    assert_eq!(audit[0].denial, Some(AuditDenial::UserCancelled));
    assert_eq!(h.bus.guard().pending_confirmations(), 0);
}

#[test]
fn the_router_sends_each_initiator_to_its_own_provider() {
    let h = Harness::new();
    h.connect("a", false);
    h.allow_deletes("a", 2);
    let ui = Scripted::new(Decision::confirm());
    let agent = Scripted::new(Decision::Decline);
    let router = ConfirmationRouter::new()
        .with(Initiator::Ui, ui.clone())
        .with(Initiator::Command, ui.clone())
        .with(Initiator::Agent, agent.clone());

    assert!(execute(&h, pod_delete("a", "web-0"), Initiator::Ui, &router).is_ok());
    assert!(execute(&h, pod_delete("a", "web-1"), Initiator::Command, &router).is_ok());
    let declined = execute(&h, pod_delete("a", "web-2"), Initiator::Agent, &router);
    assert!(matches!(declined, Err(DispatchError::Declined(_))));
    assert_eq!(
        ui.asked(),
        [
            (Initiator::Ui, ConfirmTier::Simple),
            (Initiator::Command, ConfirmTier::Simple)
        ]
    );
    assert_eq!(
        agent.asked(),
        [(Initiator::Agent, ConfirmTier::Simple)],
        "an agent's request never reaches the UI's provider"
    );

    // No provider for plugins: declined, fail closed.
    let plugin = execute(&h, pod_delete("a", "web-3"), Initiator::Plugin, &router);
    assert!(
        matches!(plugin, Err(DispatchError::Declined(_))),
        "{plugin:?}"
    );
    assert_eq!(h.writes("a").len(), 2);
}

#[test]
fn a_command_that_needs_no_confirmation_never_reaches_the_provider() {
    let h = Harness::new();
    h.connect("a", true);
    let provider = Scripted::new(Decision::confirm());
    // Read-only refuses before any question.
    let err = execute(
        &h,
        pod_delete("a", "web-0"),
        Initiator::Agent,
        provider.as_ref(),
    );
    assert!(
        matches!(err, Err(DispatchError::ReadOnly { .. })),
        "{err:?}"
    );
    assert!(provider.asked().is_empty());
}

#[test]
fn a_dropped_wait_forgets_the_request_and_audits_it_as_cancelled() {
    let h = Harness::new();
    h.connect("a", false);
    {
        let mut waiting = Box::pin(h.bus.execute(
            pod_delete("a", "web-0"),
            ctx(Initiator::Agent),
            &Silent,
        ));
        assert!(
            (&mut waiting).now_or_never().is_none(),
            "waiting on the provider"
        );
        assert_eq!(
            h.bus.guard().pending_confirmations(),
            1,
            "no lock is held while the provider waits"
        );
    }
    assert_eq!(h.bus.guard().pending_confirmations(), 0);
    assert!(h.writes("a").is_empty());
    // The record is queued synchronously; the next flush writes it.
    assert_eq!(h.bus.guard().audit().backlog_len(), 1);
    h.bus
        .guard()
        .audit()
        .flush()
        .now_or_never()
        .expect("the fake store does not wait")
        .expect("flushed");
    let audit = h.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Cancelled);
    assert_eq!(audit[0].denial, Some(AuditDenial::UserCancelled));
    assert_eq!(audit[0].initiator, Initiator::Agent);
}

#[test]
fn every_refusal_records_why() {
    let h = Harness::new();
    h.connect("a", true);
    // Read-only.
    assert!(
        h.dispatch(pod_delete("a", "web-0"), ctx(Initiator::Ui))
            .is_err()
    );
    // No open session for `b`.
    assert!(matches!(
        h.dispatch(pod_delete("b", "web-0"), ctx(Initiator::Ui)),
        Err(DispatchError::NoSession(_))
    ));
    // Declined by the user.
    h.connect("b", false);
    let request = h.ask(pod_delete("b", "web-0"), ctx(Initiator::Ui));
    h.bus
        .decline(request.token)
        .now_or_never()
        .expect("no wait")
        .expect("declined");
    let reasons: Vec<_> = h.audit().iter().map(|r| (r.outcome, r.denial)).collect();
    assert_eq!(
        reasons,
        [
            (AuditOutcome::Denied, Some(AuditDenial::ReadOnly)),
            (AuditOutcome::Denied, Some(AuditDenial::Policy)),
            (AuditOutcome::Cancelled, Some(AuditDenial::UserCancelled)),
        ]
    );
}
