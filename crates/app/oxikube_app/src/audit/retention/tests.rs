//! [`AuditRetention`] on a fake clock and `FakeStatePort`.

use std::sync::Arc;
use std::time::Duration;

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use jiff::Timestamp;
use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_ports::{ClockPort, StatePort};
use oxikube_testkit::{FakeClockPort, FakeStatePort, StateCall};

use super::{AuditRetention, PRUNE_INTERVAL};
use crate::testing::pod;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

struct Fixture {
    state: Arc<FakeStatePort>,
    clock: Arc<FakeClockPort>,
    retention: Arc<AuditRetention>,
}

fn fixture(days: u32) -> Fixture {
    let state = Arc::new(FakeStatePort::new());
    let clock = Arc::new(FakeClockPort::default());
    let retention = Arc::new(AuditRetention::new(state.clone(), clock.clone(), days));
    Fixture {
        state,
        clock,
        retention,
    }
}

fn record_at(ts: Timestamp, name: &str) -> AuditRecord {
    AuditRecord::new(
        ts,
        "alice",
        Initiator::Ui,
        "pod::Delete",
        pod("a", name),
        false,
        AuditOutcome::Succeeded,
    )
}

/// Names of the records the store still holds.
fn names(state: &FakeStatePort) -> Vec<String> {
    let mut names: Vec<String> = state
        .audit_log()
        .iter()
        .map(|r| r.target.name.to_string())
        .collect();
    names.sort();
    names
}

async fn seed(f: &Fixture) {
    let now = f.clock.now();
    let day = |n: u64| now - DAY * u32::try_from(n).unwrap();
    f.state
        .append_audit(&[
            record_at(day(120), "ancient"),
            record_at(day(91), "just-expired"),
            record_at(day(89), "fresh"),
            record_at(day(1), "yesterday"),
        ])
        .await
        .unwrap();
}

#[test]
fn rows_older_than_the_cutoff_are_removed_and_newer_kept() {
    let f = fixture(90);
    futures::executor::block_on(async {
        seed(&f).await;
        assert_eq!(f.retention.prune_now().await.unwrap(), 2);
        assert_eq!(names(&f.state), ["fresh", "yesterday"]);
        assert_eq!(f.retention.prune_now().await.unwrap(), 0);
        // The cutoff handed to the store is exactly now minus the window.
        let cutoff = f.clock.now() - DAY * 90;
        assert!(
            f.state
                .recorded_calls()
                .contains(&StateCall::PruneAudit(cutoff))
        );
    });
}

#[test]
fn zero_days_keeps_everything_and_never_calls_the_store() {
    let f = fixture(0);
    futures::executor::block_on(async {
        seed(&f).await;
        assert_eq!(f.retention.prune_now().await.unwrap(), 0);
        assert_eq!(f.state.audit_log().len(), 4);
        assert!(
            !f.state
                .recorded_calls()
                .iter()
                .any(|c| matches!(c, StateCall::PruneAudit(_)))
        );
    });
}

#[test]
fn the_window_is_clamped_to_ten_years() {
    let f = fixture(u32::MAX);
    assert_eq!(f.retention.days(), 3650);
    f.retention.set_days(u32::MAX);
    assert_eq!(f.retention.days(), 3650);
}

fn prunes(f: &Fixture) -> usize {
    f.state
        .recorded_calls()
        .iter()
        .filter(|c| matches!(c, StateCall::PruneAudit(_)))
        .count()
}

#[test]
fn run_prunes_on_start_then_daily_and_on_a_changed_window() {
    let f = fixture(90);
    let mut pool = LocalPool::new();
    futures::executor::block_on(seed(&f));
    let retention = f.retention.clone();
    pool.spawner()
        .spawn_local(async move { retention.run().await })
        .unwrap();

    pool.run_until_stalled();
    assert_eq!(prunes(&f), 1, "on start");
    assert_eq!(names(&f.state), ["fresh", "yesterday"]);

    f.clock.advance(PRUNE_INTERVAL - Duration::from_secs(1));
    pool.run_until_stalled();
    assert_eq!(prunes(&f), 1, "not before the day is out");

    f.clock.advance(Duration::from_secs(1));
    pool.run_until_stalled();
    assert_eq!(prunes(&f), 2, "daily");

    // A shorter window applies at once (hot reload), and removes the 89 day old record.
    f.retention.set_days(30);
    pool.run_until_stalled();
    assert_eq!(prunes(&f), 3);
    assert_eq!(names(&f.state), ["yesterday"]);
}

#[test]
fn a_failed_prune_is_retried_at_the_next_tick() {
    let f = fixture(90);
    let mut pool = LocalPool::new();
    futures::executor::block_on(seed(&f));
    f.state
        .script()
        .prune_audit
        .push_err(OxiError::internal("disk full"));
    let retention = f.retention.clone();
    pool.spawner()
        .spawn_local(async move { retention.run().await })
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(
        f.state.audit_log().len(),
        4,
        "the failed prune removed nothing"
    );
    f.clock.advance(PRUNE_INTERVAL);
    pool.run_until_stalled();
    assert_eq!(f.state.audit_log().len(), 2);
}

#[test]
fn unchanged_window_does_not_wake_the_scheduler() {
    let f = fixture(90);
    let mut pool = LocalPool::new();
    let retention = f.retention.clone();
    pool.spawner()
        .spawn_local(async move { retention.run().await })
        .unwrap();
    pool.run_until_stalled();
    f.retention.set_days(90);
    pool.run_until_stalled();
    assert_eq!(prunes(&f), 1);
}
