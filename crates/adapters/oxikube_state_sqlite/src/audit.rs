//! SQL for the append-only `audit_log` table.

use jiff::Timestamp;
use oxikube_domain::{OxiError, OxiResult, audit::AuditRecord};
use oxikube_ports::AuditQuery;
use rusqlite::{Connection, TransactionBehavior, params};

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

/// Records matching `query`, newest first (`since` inclusive, `until` exclusive), then `limit`.
/// Records with equal timestamps come back last-appended first.
pub(crate) fn query(conn: &Connection, query: &AuditQuery) -> OxiResult<Vec<AuditRecord>> {
    let limit = i64::try_from(query.limit).unwrap_or(i64::MAX);
    let mut stmt = conn
        .prepare_cached(
            "SELECT record FROM audit_log
             WHERE (?1 IS NULL OR cluster = ?1)
               AND (?2 IS NULL OR ts_ns >= ?2)
               AND (?3 IS NULL OR ts_ns < ?3)
               AND (?4 IS NULL OR cmd = ?4)
             ORDER BY ts_ns DESC, id DESC
             LIMIT ?5",
        )
        .map_err(oxi)?;
    let rows = stmt
        .query_map(
            params![
                query.cluster.as_ref().map(|c| c.as_str()),
                query.since.map(nanos),
                query.until.map(nanos),
                query.cmd.as_deref(),
                limit,
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(oxi)?;
    let mut out = Vec::new();
    for row in rows {
        let json = row.map_err(oxi)?;
        match serde_json::from_str::<AuditRecord>(&json) {
            Ok(record) => out.push(record),
            // One unreadable row (written by a newer build, say) must not hide the rest.
            Err(e) => tracing::warn!(error = %e, "skipping an unreadable audit record"),
        }
    }
    Ok(out)
}
