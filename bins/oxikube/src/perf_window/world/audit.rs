//! The `audit-log` scenario's state database: a real SQLite file seeded with 100 000 audit
//! records, so the viewer's paged, filtered reads cost what the app's do (the in-memory fake
//! sorts the whole log per query and would measure itself).

use std::path::PathBuf;
use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::safety::MutationVerb;
use oxikube_ports::StatePort;
use oxikube_state_sqlite::SqliteState;

use super::WorldSpec;

/// Records per insert batch (one transaction each).
const BATCH: usize = 5_000;

/// The database file: replaced on every run, in the OS temp directory (never the user's).
fn path() -> PathBuf {
    std::env::temp_dir().join("oxikube-perf-audit.sqlite3")
}

/// The `n`th record, `n` seconds before `now`, about the main cluster's pods.
fn record(spec: &WorldSpec, now: Timestamp, n: usize) -> AuditRecord {
    const COMMANDS: [&str; 5] = [
        "pod::Delete",
        "workload::Scale",
        "workload::Restart",
        "resource::Apply",
        "node::Cordon",
    ];
    let cluster = spec
        .clusters
        .first()
        .map_or_else(|| super::cluster_id("perf-a"), |c| c.cluster_id());
    let cmd = COMMANDS[n % COMMANDS.len()];
    let target = ResourceRef::namespaced(
        cluster,
        Gvk::from_api_version("v1", "Pod"),
        super::namespace_name(n % 8),
        format!("load-{:05}", n % 10_000),
    );
    let outcome = match n % 7 {
        0 => AuditOutcome::Failed,
        1 => AuditOutcome::Denied,
        _ => AuditOutcome::Succeeded,
    };
    let initiator = Initiator::ALL[n % Initiator::ALL.len()];
    let at = now - SignedDuration::from_secs(i64::try_from(n).unwrap_or(0));
    let record = AuditRecord::new(
        at,
        "perf",
        initiator,
        cmd,
        target,
        n.is_multiple_of(11),
        outcome,
    );
    match MutationVerb::for_command(cmd) {
        Some(verb) => record.with_verb(verb),
        None => record,
    }
}

/// A SQLite state database holding `spec.audit_records` records.
///
/// # Panics
///
/// When the database cannot be created or seeded: a perf run that cannot seed its load must not
/// run.
pub fn seeded_state(spec: &WorldSpec) -> Arc<dyn StatePort> {
    let path = path();
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.clone().into_os_string();
        file.push(suffix);
        std::fs::remove_file(PathBuf::from(file)).ok();
    }
    let state = futures::executor::block_on(SqliteState::open(&path))
        .expect("the perf audit database opens");
    let now = Timestamp::now();
    let mut next = 0;
    while next < spec.audit_records {
        let end = (next + BATCH).min(spec.audit_records);
        let batch: Vec<AuditRecord> = (next..end).map(|n| record(spec, now, n)).collect();
        futures::executor::block_on(state.append_audit(&batch))
            .expect("the perf audit database takes its records");
        next = end;
    }
    Arc::new(state)
}
