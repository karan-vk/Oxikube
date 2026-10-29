//! The viewer's model against `FakeStatePort`: filters become store queries, pages cover the log
//! exactly once, the "N new" count.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::safety::MutationVerb;
use oxikube_testkit::FakeStatePort;

use super::*;
use crate::testing::pod;

fn base() -> Timestamp {
    "2026-10-30T12:00:00Z".parse().expect("a valid timestamp")
}

fn record(n: i64, cmd: &str, initiator: Initiator, outcome: AuditOutcome) -> AuditRecord {
    let mut record = AuditRecord::new(
        base().checked_add(n.seconds()).expect("in range"),
        "alice",
        initiator,
        cmd,
        pod("a", &format!("web-{n}")),
        false,
        outcome,
    );
    if let Some(verb) = MutationVerb::for_command(cmd) {
        record = record.with_verb(verb);
    }
    record
}

async fn seeded(count: i64) -> (Arc<FakeStatePort>, AuditReader) {
    let state = Arc::new(FakeStatePort::new());
    let records: Vec<_> = (0..count)
        .map(|n| {
            let cmd = if n % 3 == 0 {
                "pod::Delete"
            } else {
                "workload::Scale"
            };
            let initiator = if n % 2 == 0 {
                Initiator::Ui
            } else {
                Initiator::Agent
            };
            record(n, cmd, initiator, AuditOutcome::Succeeded)
        })
        .collect();
    state.append_audit(&records).await.expect("seeds");
    let reader = AuditReader::new(state.clone());
    (state, reader)
}

#[tokio::test]
async fn pages_cover_the_log_newest_first_exactly_once() {
    let (_state, reader) = seeded(450).await;
    let filters = AuditFilters::default();
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader
            .page(&filters, base(), cursor, PAGE_SIZE)
            .await
            .expect("page");
        assert!(page.records.len() <= PAGE_SIZE);
        seen.extend(page.records);
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 450);
    assert!(seen.windows(2).all(|w| w[0].ts >= w[1].ts), "newest first");
    let mut names: Vec<_> = seen.iter().map(|r| r.target.name.clone()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 450, "no record twice");
}

#[tokio::test]
async fn filters_are_pushed_into_the_store_query() {
    let (state, reader) = seeded(60).await;
    let filters = AuditFilters {
        verb: Some(MutationVerb::Delete),
        initiator: Some(Initiator::Ui),
        ..AuditFilters::default()
    };
    let page = reader
        .page(&filters, base(), None, PAGE_SIZE)
        .await
        .expect("page");
    assert_eq!(page.records.len(), 10, "every sixth record is a Ui delete");
    assert!(
        page.records
            .iter()
            .all(|r| { r.verb == Some(MutationVerb::Delete) && r.initiator == Initiator::Ui })
    );
    let calls = state.recorded_calls();
    let Some(oxikube_testkit::StateCall::QueryAudit(query)) = calls.last() else {
        panic!("a query reached the store");
    };
    assert_eq!(query.verb, Some(MutationVerb::Delete));
    assert_eq!(query.initiator, Some(Initiator::Ui));
}

#[tokio::test]
async fn the_time_window_is_a_since_bound() {
    let (_state, reader) = seeded(10).await;
    let now = base().checked_add(100.seconds()).expect("in range");
    let filters = AuditFilters {
        window: AuditWindow::LastHour,
        ..AuditFilters::default()
    };
    assert_eq!(
        filters.query(now, None, 5).since,
        now.checked_sub(1.hour()).ok()
    );
    assert_eq!(AuditFilters::default().query(now, None, 5).since, None);
    let later = base().checked_add(3.hours()).expect("in range");
    let page = reader
        .page(&filters, later, None, PAGE_SIZE)
        .await
        .expect("page");
    assert!(page.records.is_empty(), "everything is older than an hour");
}

#[tokio::test]
async fn a_batch_lists_its_records_only() {
    let state = Arc::new(FakeStatePort::new());
    let mut records: Vec<_> = (0..5)
        .map(|n| {
            record(n, "pod::Delete", Initiator::Ui, AuditOutcome::Succeeded).with_batch_id("b-1")
        })
        .collect();
    records.push(record(
        9,
        "pod::Delete",
        Initiator::Ui,
        AuditOutcome::Succeeded,
    ));
    state.append_audit(&records).await.expect("seeds");
    let batch = AuditReader::new(state).batch("b-1").await.expect("batch");
    assert_eq!(batch.len(), 5);
    assert!(batch.iter().all(|r| r.batch_id.as_deref() == Some("b-1")));
}

#[test]
fn new_records_are_counted_against_the_head() {
    let fresh: Vec<_> = (0..5)
        .rev()
        .map(|n| record(n, "pod::Delete", Initiator::Ui, AuditOutcome::Succeeded))
        .collect();
    assert_eq!(count_new(Some(&fresh[2]), &fresh), NewRecords::Exactly(2));
    assert_eq!(count_new(Some(&fresh[0]), &fresh), NewRecords::Exactly(0));
    let gone = record(-1, "pod::Delete", Initiator::Ui, AuditOutcome::Failed);
    assert_eq!(count_new(Some(&gone), &fresh), NewRecords::AtLeast(5));
    // A head a sliding time window aged out: an empty or older-only page is no news.
    let aged_out = record(99, "pod::Delete", Initiator::Ui, AuditOutcome::Failed);
    assert_eq!(count_new(Some(&aged_out), &[]), NewRecords::Exactly(0));
    assert_eq!(count_new(Some(&aged_out), &fresh), NewRecords::Exactly(0));
    let mid = record(2, "pod::Other", Initiator::Ui, AuditOutcome::Failed);
    assert_eq!(count_new(Some(&mid), &fresh), NewRecords::Exactly(2));
    assert_eq!(count_new(None, &fresh), NewRecords::Exactly(5));
    assert_eq!(NewRecords::AtLeast(5).label(), "5+");
    assert!(NewRecords::Exactly(0).is_none());
}
