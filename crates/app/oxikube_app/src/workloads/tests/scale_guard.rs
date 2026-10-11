//! `workload::Scale` through the guard: tier, read-only, audit with before and after.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_testkit::ResourceCall;

use super::{deployment, scale, scale_to};
use crate::command_bus::DispatchError;
use crate::testing::{Harness, ctx};

#[test]
fn scaling_to_a_count_is_low_and_asks_once() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    for replicas in [1, 5, 100] {
        let request = h.ask(scale_to(deployment("a"), replicas), ctx(Initiator::Ui));
        assert_eq!(request.risk, Some(Risk::Low), "{replicas}");
        assert_eq!(request.tier, ConfirmTier::Simple, "{replicas}");
        assert_eq!(request.expected_name, None, "{replicas}");
    }
    assert!(
        h.writes("a").is_empty(),
        "nothing is sent before the answer"
    );
}

#[test]
fn scaling_to_zero_is_medium_and_says_every_pod_stops() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    let request = h.ask(scale_to(deployment("a"), 0), ctx(Initiator::Ui));
    assert_eq!(request.risk, Some(Risk::Medium));
    // Medium still maps to one simple confirmation, naming the target and the cluster.
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert!(
        request
            .summary
            .starts_with("Scale Deployment shop/web to 0 replicas (stops all of its pods) on "),
        "{}",
        request.summary
    );
    let one = h.ask(scale_to(deployment("a"), 1), ctx(Initiator::Ui));
    assert!(
        one.summary
            .starts_with("Scale Deployment shop/web to 1 replica on "),
        "{}",
        one.summary
    );
}

#[test]
fn the_tier_comes_from_the_intent_not_the_dialog() {
    // The same command, from any door, gets the same tier.
    let h = Harness::with_workloads();
    h.connect("a", false);
    for initiator in Initiator::ALL {
        let low = h.ask(scale_to(deployment("a"), 3), ctx(initiator));
        let zero = h.ask(scale_to(deployment("a"), 0), ctx(initiator));
        assert_eq!(
            (low.risk, zero.risk),
            (Some(Risk::Low), Some(Risk::Medium)),
            "{initiator}"
        );
    }
}

#[test]
fn a_read_only_cluster_refuses_every_initiator_before_any_request() {
    let h = Harness::with_workloads();
    h.connect("a", true);
    for initiator in Initiator::ALL {
        let error = h
            .dispatch(scale_to(deployment("a"), 3), ctx(initiator))
            .unwrap_err();
        assert!(
            matches!(error, DispatchError::ReadOnly { .. }),
            "{initiator}"
        );
    }
    assert!(
        h.resources("a").recorded_calls().is_empty(),
        "no request of any kind"
    );
    assert_eq!(h.audit().len(), Initiator::ALL.len());
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn the_audit_record_names_the_workload_the_initiator_and_before_and_after() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a").script().get_scale.push_ok(scale(3, 3));
    h.resources("a").script().scale.push_ok(scale(5, 3));
    h.confirm_and_run(scale_to(deployment("a"), 5), ctx(Initiator::Ui))
        .unwrap();
    let audit = h.audit();
    assert_eq!(audit.len(), 1);
    let record = &audit[0];
    assert_eq!(&*record.cmd, "workload::Scale");
    assert_eq!(record.target, deployment("a"));
    assert_eq!(record.initiator, Initiator::Ui);
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(
        record.detail.as_deref(),
        Some("replicas=5 before=3 after=5")
    );
}

#[test]
fn a_failed_scale_is_audited_with_what_it_started_from() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a").script().get_scale.push_ok(scale(3, 3));
    h.resources("a")
        .script()
        .scale
        .push_err(oxikube_domain::OxiError::forbidden(
            "scale denied by admission",
        ));
    let error = h
        .confirm_and_run(scale_to(deployment("a"), 0), ctx(Initiator::Agent))
        .unwrap_err();
    assert!(matches!(error, DispatchError::Handler(_)));
    let record = &h.audit()[0];
    assert_eq!(record.outcome, AuditOutcome::Failed);
    assert_eq!(record.initiator, Initiator::Agent);
    assert_eq!(record.detail.as_deref(), Some("replicas=0 before=3"));
}

#[test]
fn declining_the_confirmation_scales_nothing_and_is_audited_as_cancelled() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    let request = h.ask(scale_to(deployment("a"), 0), ctx(Initiator::Ui));
    futures::executor::block_on(h.bus.decline(request.token)).unwrap();
    assert!(h.writes("a").is_empty());
    let record = &h.audit()[0];
    assert_eq!(record.outcome, AuditOutcome::Cancelled);
    assert_eq!(record.detail.as_deref(), Some("replicas=0"));
}

#[test]
fn only_one_scale_request_is_written() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a").script().get_scale.push_ok(scale(1, 1));
    h.resources("a").script().scale.push_ok(scale(0, 1));
    h.confirm_and_run(scale_to(deployment("a"), 0), ctx(Initiator::Ui))
        .unwrap();
    assert!(matches!(
        h.writes("a").as_slice(),
        [ResourceCall::Scale { replicas: 0, .. }]
    ));
}
