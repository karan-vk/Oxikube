//! SQL for the `kv` table: the free-form kv store (`namespace = ''`) and the typed tables
//! (`namespace = <table name>`).

use oxikube_domain::{OxiError, OxiResult};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::{error::oxi, now_millis};

/// The namespace of the kv store. Typed tables cannot clash with it: `StateTable` names are
/// never empty.
pub(crate) const KV_NAMESPACE: &str = "";

fn parse(text: String) -> OxiResult<Value> {
    serde_json::from_str(&text)
        .map_err(|e| OxiError::internal("a stored state value is not valid JSON").with_source(e))
}

pub(crate) fn get(conn: &Connection, ns: &str, key: &str) -> OxiResult<Option<Value>> {
    conn.query_row(
        "SELECT value FROM kv WHERE namespace = ?1 AND key = ?2",
        params![ns, key],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(oxi)?
    .map(parse)
    .transpose()
}

pub(crate) fn set(conn: &Connection, ns: &str, key: &str, value: &Value) -> OxiResult<()> {
    let text = serde_json::to_string(value)
        .map_err(|e| OxiError::validation("value cannot be stored as state").with_source(e))?;
    conn.execute(
        "INSERT INTO kv (namespace, key, value, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (namespace, key) DO UPDATE SET value = ?3, updated_at = ?4",
        params![ns, key, text, now_millis()],
    )
    .map_err(oxi)?;
    Ok(())
}

pub(crate) fn delete(conn: &Connection, ns: &str, key: &str) -> OxiResult<bool> {
    let rows = conn
        .execute(
            "DELETE FROM kv WHERE namespace = ?1 AND key = ?2",
            params![ns, key],
        )
        .map_err(oxi)?;
    Ok(rows > 0)
}

/// Rows of `ns` whose key starts with `prefix`, ordered by key (byte order, like `String`'s
/// `Ord`), at most `limit`.
pub(crate) fn list(
    conn: &Connection,
    ns: &str,
    prefix: &str,
    limit: Option<usize>,
) -> OxiResult<Vec<(String, Value)>> {
    let limit = limit.map_or(-1, |l| i64::try_from(l).unwrap_or(i64::MAX));
    // `key >= prefix` lets the primary key seek; the `substr` test is the exact prefix match
    // (no LIKE, so `%` and `_` in a prefix mean themselves).
    let mut stmt = conn
        .prepare_cached(
            "SELECT key, value FROM kv
             WHERE namespace = ?1 AND key >= ?2 AND substr(key, 1, length(?2)) = ?2
             ORDER BY key LIMIT ?3",
        )
        .map_err(oxi)?;
    let rows = stmt
        .query_map(params![ns, prefix, limit], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(oxi)?;
    rows.map(|row| {
        let (key, text) = row.map_err(oxi)?;
        Ok((key, parse(text)?))
    })
    .collect()
}
