//! [`AuditLog`] against `FakeStatePort`: order, backlog, bound, redaction, timestamps.

use std::sync::Arc;

use futures::FutureExt;
use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_ports::ClockPort;
use oxikube_testkit::{FakeClockPort, FakeStatePort, StateCall};
use std::time::Duration;

use super::{AuditLog, MAX_AUDIT_BACKLOG};
use crate::testing::pod;

struct Fixture {
    state: Arc<FakeStatePort>,
    clock: Arc<FakeClockPort>,
    log: AuditLog,
}

fn fixture() -> Fixture {
    let state = Arc::new(FakeStatePort::new());
    let clock = Arc::new(FakeClockPort::default());
    let log = AuditLog::new(state.clone(), clock.clone());
    Fixture { state, clock, log }
}

fn entry(log: &AuditLog, name: &str) -> AuditRecord {
    log.entry(
        "alice",
        Initiator::Ui,
        "pod::Delete",
        pod("a", name),
        false,
        AuditOutcome::Succeeded,
    )
}

fn fail_next(state: &FakeStatePort) {
    state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
}

#[test]
fn entries_are_stamped_by_the_clock_and_redacted() {
    let f = fixture();
    let record = f.log.entry(
        "token=s3cr3t",
        Initiator::Plugin,
        "pod::Delete",
        pod("a", "web-0"),
        true,
        AuditOutcome::Denied,
    );
    assert_eq!(record.ts, f.clock.now());
    assert!(!record.who.contains("s3cr3t"));
    assert_eq!(record.initiator, Initiator::Plugin);
    assert!(record.dry_run);
}

#[test]
fn records_are_appended_in_order_after_the_backlog() {
    let f = fixture();
    fail_next(&f.state);
    assert!(
        f.log
            .record(entry(&f.log, "one"))
            .now_or_never()
            .unwrap()
            .is_err()
    );
    assert_eq!(f.log.backlog_len(), 1);

    f.log
        .record(entry(&f.log, "two"))
        .now_or_never()
        .unwrap()
        .unwrap();
    let names: Vec<_> = f
        .state
        .audit_log()
        .iter()
        .map(|r| r.target.name.to_string())
        .collect();
    assert_eq!(names, ["one", "two"]);
    assert_eq!(f.log.backlog_len(), 0);
    let batches: Vec<_> = f
        .state
        .recorded_calls()
        .into_iter()
        .filter_map(|c| match c {
            StateCall::AppendAudit(batch) => Some(batch.len()),
            _ => None,
        })
        .collect();
    assert_eq!(batches, [1, 2], "the backlog is retried in one batch");
}

#[test]
fn ensure_writable_reports_a_failing_store_and_is_cheap_when_empty() {
    let f = fixture();
    f.log.ensure_writable().now_or_never().unwrap().unwrap();
    assert!(
        f.state.recorded_calls().is_empty(),
        "nothing to flush, no call"
    );

    fail_next(&f.state);
    let _ = f.log.record(entry(&f.log, "one")).now_or_never().unwrap();
    fail_next(&f.state);
    assert!(f.log.ensure_writable().now_or_never().unwrap().is_err());
    f.log.ensure_writable().now_or_never().unwrap().unwrap();
    assert_eq!(f.state.audit_log().len(), 1);
}

#[test]
fn the_backlog_is_bounded() {
    let f = fixture();
    let extra = 10;
    for i in 0..MAX_AUDIT_BACKLOG + extra {
        fail_next(&f.state);
        let _ = f
            .log
            .record(entry(&f.log, &format!("p{i}")))
            .now_or_never()
            .unwrap();
    }
    assert_eq!(f.log.backlog_len(), MAX_AUDIT_BACKLOG);
    f.log.ensure_writable().now_or_never().unwrap().unwrap();
    let log = f.state.audit_log();
    assert_eq!(log.len(), MAX_AUDIT_BACKLOG);
    assert_eq!(
        log[0].target.name.as_ref(),
        format!("p{extra}"),
        "with no denied records to drop, the oldest were dropped"
    );
}

#[test]
fn overflow_drops_denied_records_before_a_mutation_record() {
    let f = fixture();
    // The mutation whose own flush failed: its record is the oldest in the backlog.
    fail_next(&f.state);
    let _ = f.log.record(entry(&f.log, "ran")).now_or_never().unwrap();
    // A long outage: many denied dispatches queue behind it.
    for i in 0..MAX_AUDIT_BACKLOG + 10 {
        fail_next(&f.state);
        let denied = f.log.entry(
            "alice",
            Initiator::Agent,
            "pod::Delete",
            pod("a", &format!("d{i}")),
            false,
            AuditOutcome::Denied,
        );
        let _ = f.log.record(denied).now_or_never().unwrap();
    }
    assert_eq!(f.log.backlog_len(), MAX_AUDIT_BACKLOG);

    f.log.ensure_writable().now_or_never().unwrap().unwrap();
    let log = f.state.audit_log();
    assert_eq!(log.len(), MAX_AUDIT_BACKLOG);
    assert_eq!(
        log[0].target.name.as_ref(),
        "ran",
        "the mutation record survives"
    );
    assert_eq!(log[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(
        log[1].target.name.as_ref(),
        "d11",
        "the oldest denied records were dropped instead"
    );
}

fn begin<'a>(log: &'a AuditLog, name: &str) -> super::AuditAttempt<'a> {
    log.begin(
        "alice",
        Initiator::Agent,
        "pod::Delete",
        pod("a", name),
        true,
    )
}

#[test]
fn a_dropped_attempt_is_queued_as_cancelled() {
    let f = fixture();
    drop(begin(&f.log, "web-0"));
    assert_eq!(f.log.backlog_len(), 1, "queued without any await");
    assert!(f.state.recorded_calls().is_empty());

    f.log.ensure_writable().now_or_never().unwrap().unwrap();
    let log = f.state.audit_log();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].outcome, AuditOutcome::Cancelled);
    assert_eq!(log[0].initiator, Initiator::Agent);
    assert!(log[0].dry_run);
}

#[test]
fn a_finished_attempt_carries_its_outcome_and_end_time() {
    let f = fixture();
    let attempt = begin(&f.log, "web-0");
    f.clock.advance(Duration::from_secs(5));
    attempt.finish(AuditOutcome::Failed);
    assert_eq!(f.log.backlog_len(), 1);
    f.log.flush().now_or_never().unwrap().unwrap();
    let log = f.state.audit_log();
    assert_eq!(log.len(), 1, "finishing disarms the drop record");
    assert_eq!(log[0].outcome, AuditOutcome::Failed);
    assert_eq!(log[0].ts, f.clock.now());
}

#[test]
fn a_record_is_queued_even_if_its_write_is_never_polled() {
    let f = fixture();
    drop(f.log.record(entry(&f.log, "one")));
    assert_eq!(f.log.backlog_len(), 1);
    f.log.ensure_writable().now_or_never().unwrap().unwrap();
    assert_eq!(f.state.audit_log().len(), 1);
}
