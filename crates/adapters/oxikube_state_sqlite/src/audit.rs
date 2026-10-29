//! SQL for the append-only `audit_log` table: appends, keyset-paged queries and retention pruning.

use jiff::Timestamp;
use oxikube_domain::{OxiError, OxiResult, audit::AuditRecord};
use oxikube_ports::{AuditCursor, AuditPage, AuditQuery};
use rusqlite::{Connection, TransactionBehavior, params, params_from_iter, types::Value};
use serde::Serialize;

use crate::error::oxi;

/// A timestamp as nanoseconds since the epoch, saturating at the `i64` range (about the year
/// 2262), which keeps ordering and range filters exact for any realistic record.
fn nanos(ts: Timestamp) -> i64 {
    i64::try_from(ts.as_nanosecond()).unwrap_or(if ts.as_nanosecond() < 0 {
        i64::MIN
    } else {
        i64::MAX
    })
}

/// Appends `records` in one transaction: all of them or none.
pub(crate) fn append(conn: &mut Connection, records: &[AuditRecord]) -> OxiResult<()> {
    if records.is_empty() {
        return Ok(());
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(oxi)?;
    {
        let mut insert = tx
            .prepare_cached(
                "INSERT INTO audit_log (ts_ns, cluster, cmd, record) VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(oxi)?;
        for record in records {
            let json = serde_json::to_string(record).map_err(|e| {
                OxiError::validation("audit record cannot be stored").with_source(e)
            })?;
            insert
                .execute(params![
                    nanos(record.ts),
                    record.cluster.as_str(),
                    &*record.cmd,
                    json
                ])
                .map_err(oxi)?;
        }
    }
    tx.commit().map_err(oxi)
}

/// The stored name of a unit enum (its serde form), which is what the generated columns hold.
fn name_of<T: Serialize>(value: &T) -> OxiResult<String> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => Ok(name),
        _ => Err(OxiError::internal("audit filter has no stored name")),
    }
}

/// The `WHERE` clause and parameters of `query`: one equality or range per set filter, so each
/// filter that has an index can use it (a `? IS NULL OR ...` form would defeat them all).
pub(crate) fn filter(query: &AuditQuery) -> OxiResult<(String, Vec<Value>)> {
    let mut clauses: Vec<&str> = Vec::new();
    let mut args: Vec<Value> = Vec::new();
    let mut add = |clause: &'static str, values: &[Value]| {
        clauses.push(clause);
        args.extend_from_slice(values);
    };
    if let Some(cluster) = &query.cluster {
        add("cluster = ?", &[Value::Text(cluster.as_str().to_owned())]);
    }
    if let Some(since) = query.since {
        add("ts_ns >= ?", &[Value::Integer(nanos(since))]);
    }
    if let Some(until) = query.until {
        add("ts_ns < ?", &[Value::Integer(nanos(until))]);
    }
    if let Some(cmd) = &query.cmd {
        add("cmd = ?", &[Value::Text(cmd.clone())]);
    }
    if let Some(verb) = query.verb {
        add("verb = ?", &[Value::Text(verb.as_str().to_owned())]);
    }
    if let Some(initiator) = query.initiator {
        add("initiator = ?", &[Value::Text(name_of(&initiator)?)]);
    }
    if let Some(outcome) = query.outcome {
        add("outcome = ?", &[Value::Text(name_of(&outcome)?)]);
    }
    if let Some(batch_id) = &query.batch_id {
        add("batch_id = ?", &[Value::Text(batch_id.clone())]);
    }
    if let Some(cursor) = query.cursor {
        // Strictly after the cursor in `(ts_ns DESC, id DESC)` order.
        let (ts, row) = (
            nanos(cursor.ts()),
            i64::try_from(cursor.row()).unwrap_or(i64::MAX),
        );
        add(
            "(ts_ns < ? OR (ts_ns = ? AND id < ?))",
            &[Value::Integer(ts), Value::Integer(ts), Value::Integer(row)],
        );
    }
    let clause = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    Ok((clause, args))
}

/// The page query for a `WHERE` clause from [`filter`].
pub(crate) fn select_sql(clause: &str) -> String {
    format!("SELECT id, ts_ns, record FROM audit_log {clause} ORDER BY ts_ns DESC, id DESC LIMIT ?")
}

/// One page of the records matching `query`, newest first (`since` inclusive, `until`
/// exclusive); records with equal timestamps come back last-appended first. The page holds at
/// most `limit` records, and `next` is set when older ones remain.
pub(crate) fn query(conn: &Connection, query: &AuditQuery) -> OxiResult<AuditPage> {
    let limit = query.limit.min(MAX_PAGE);
    let (clause, mut args) = filter(query)?;
    // One extra row tells whether another page follows without a second query.
    args.push(Value::Integer(i64::try_from(limit + 1).unwrap_or(i64::MAX)));
    let sql = select_sql(&clause);
    let mut stmt = conn.prepare_cached(&sql).map_err(oxi)?;
    let rows = stmt
        .query_map(params_from_iter(args), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(oxi)?;
    let mut page = AuditPage::default();
    let mut last = None;
    for (taken, row) in rows.enumerate() {
        let (id, ts_ns, json) = row.map_err(oxi)?;
        if taken == limit {
            // The extra row: more follow, starting after the last row of this page.
            page.next = last.map(|(ts, row)| AuditCursor::new(timestamp(ts), row));
            break;
        }
        last = Some((ts_ns, u64::try_from(id).unwrap_or(0)));
        match serde_json::from_str::<AuditRecord>(&json) {
            Ok(record) => page.records.push(record),
            // One unreadable row (written by a newer build, say) must not hide the rest.
            Err(e) => tracing::warn!(error = %e, "skipping an unreadable audit record"),
        }
    }
    Ok(page)
}

/// Largest page, so a careless `usize::MAX` limit cannot ask SQLite for an unbounded result.
const MAX_PAGE: usize = 100_000;

fn timestamp(ns: i64) -> Timestamp {
    Timestamp::from_nanosecond(i128::from(ns)).unwrap_or(Timestamp::MAX)
}

/// Removes the records stamped before `before` and gives the space back. Returns how many went.
///
/// The delete trigger of migration 0002 refuses any row at or after the cutoff parked in
/// `audit_prune`, so a wrong clause cannot remove a record that is still in its retention.
pub(crate) fn prune(conn: &mut Connection, before: Timestamp) -> OxiResult<u64> {
    let cutoff = nanos(before);
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(oxi)?;
    tx.execute("DELETE FROM audit_prune", []).map_err(oxi)?;
    tx.execute("INSERT INTO audit_prune (cutoff_ns) VALUES (?1)", [cutoff])
        .map_err(oxi)?;
    let removed = tx
        .execute("DELETE FROM audit_log WHERE ts_ns < ?1", [cutoff])
        .map_err(oxi)?;
    tx.execute("DELETE FROM audit_prune", []).map_err(oxi)?;
    tx.commit().map_err(oxi)?;
    if removed > 0 {
        reclaim(conn)?;
    }
    Ok(removed as u64)
}

/// Returns the freed pages to the file system. A database created by this build is in
/// incremental auto-vacuum mode, so the free pages go back with `incremental_vacuum`. An older
/// file is in `NONE` mode, which only a full `VACUUM` can change: it is converted once, here,
/// off the UI thread, and is incremental from then on.
fn reclaim(conn: &Connection) -> OxiResult<()> {
    let mode: i64 = conn
        .query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
        .map_err(oxi)?;
    if mode == AUTO_VACUUM_INCREMENTAL {
        conn.execute_batch("PRAGMA incremental_vacuum")
            .map_err(oxi)?;
    } else {
        conn.execute_batch(&format!(
            "PRAGMA auto_vacuum = {AUTO_VACUUM_INCREMENTAL}; VACUUM"
        ))
        .map_err(oxi)?;
    }
    // Hand the WAL's pages back too; best effort (a reader may hold it).
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    Ok(())
}

/// `PRAGMA auto_vacuum` value for incremental mode.
pub(crate) const AUTO_VACUUM_INCREMENTAL: i64 = 2;
