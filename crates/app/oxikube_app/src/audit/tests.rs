//! [`AuditLog`] against `FakeStatePort`: order, backlog, bound, redaction, timestamps.

use std::sync::Arc;

use futures::FutureExt;
use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_ports::ClockPort;
use oxikube_testkit::{FakeClockPort, FakeStatePort, StateCall};

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
        "the oldest were dropped"
    );
}
