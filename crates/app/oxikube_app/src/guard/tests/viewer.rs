//! Epic E19 done-when 4: a mutation that went through the guard is readable by the audit viewer
//! (`AuditReader`, the viewer's model) and announced to it (`AuditLog::appended`) within a second
//! of the guard returning.

use std::time::{Duration, Instant};

use futures::FutureExt as _;
use jiff::Timestamp;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::safety::MutationVerb;
use oxikube_ports::ClockPort as _;

use crate::audit::query::{AuditFilters, AuditReader, PAGE_SIZE};
use crate::command_bus::Outcome;
use crate::testing::{Harness, ctx, id, pod, pod_delete};

#[test]
fn a_guarded_mutation_is_in_the_viewers_model_within_a_second() {
    let h = Harness::new();
    h.connect("a", false);
    h.allow_deletes("a", 1);
    let listener = h.bus.guard().audit().appended();
    let reader = AuditReader::new(h.state.clone());
    let now: Timestamp = h.clock.now();

    let started = Instant::now();
    let out = h.confirm_and_run(pod_delete("a", "web-0"), ctx(Initiator::Agent));
    assert!(matches!(out, Ok(Outcome::Completed(_))), "{out:?}");

    assert!(listener.has_pending(), "the flush told the viewer");
    let page = reader
        .page(&AuditFilters::default(), now, None, PAGE_SIZE)
        .now_or_never()
        .expect("the fake answers at once")
        .expect("reads");
    assert!(started.elapsed() < Duration::from_secs(1));
    let record = page.records.first().expect("the mutation is listed");
    assert_eq!(record.target, pod("a", "web-0"));
    assert_eq!(record.cluster, id("a"));
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(record.initiator, Initiator::Agent);
    assert_eq!(record.verb, Some(MutationVerb::Delete));
}

#[test]
fn a_denied_attempt_is_listed_too_and_filters_find_it() {
    let h = Harness::new();
    h.connect("a", true);
    let reader = AuditReader::new(h.state.clone());
    assert!(
        h.dispatch(pod_delete("a", "web-0"), ctx(Initiator::Ui))
            .is_err()
    );
    let filters = AuditFilters {
        outcome: Some(AuditOutcome::Denied),
        cluster: Some(id("a")),
        ..AuditFilters::default()
    };
    let page = reader
        .page(&filters, h.clock.now(), None, PAGE_SIZE)
        .now_or_never()
        .expect("answers at once")
        .expect("reads");
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].outcome, AuditOutcome::Denied);
}
