//! The schema migrations: an ordered list of SQL scripts embedded in the binary, applied in a
//! transaction each when the database opens, and recorded in the `migrations` table.
//!
//! Rules for adding one: append a [`Migration`] with the next version and a new
//! `NNNN_name.sql`; never edit or reorder an applied one (a shipped database has already run
//! it). A database whose recorded version is newer than the last migration here was written by a
//! newer Oxikube: opening it is refused instead of guessing at its schema.

use rusqlite::{Connection, TransactionBehavior};

use crate::error::StateFailure;

/// One schema step.
pub struct Migration {
    /// 1-based, strictly increasing, gap-free.
    pub version: u32,
    /// Short name, recorded with the version.
    pub name: &'static str,
    /// The SQL, run as one batch inside the migration's transaction.
    pub sql: &'static str,
}

/// Every migration, oldest first.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "init",
    sql: include_str!("0001_init.sql"),
}];

/// The schema version a fully migrated database has.
pub fn latest_version() -> u32 {
    latest_of(MIGRATIONS)
}

fn latest_of(migrations: &[Migration]) -> u32 {
    migrations.last().map_or(0, |m| m.version)
}

const CREATE_MIGRATIONS_TABLE: &str = "CREATE TABLE IF NOT EXISTS migrations (
    version    INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    applied_at INTEGER NOT NULL
)";

/// Brings `conn` up to [`latest_version`], returning the version it ends at. Idempotent: applied
/// migrations are skipped, so opening a migrated database changes nothing.
pub fn apply(conn: &mut Connection) -> Result<u32, StateFailure> {
    apply_all(conn, MIGRATIONS)
}

/// [`apply`] over an explicit list, so a test can run a migration that fails.
fn apply_all(conn: &mut Connection, migrations: &[Migration]) -> Result<u32, StateFailure> {
    let latest = latest_of(migrations);
    conn.execute_batch(CREATE_MIGRATIONS_TABLE)?;
    let current: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM migrations",
        [],
        |r| r.get(0),
    )?;
    if current > latest {
        return Err(StateFailure::Newer {
            found: current,
            supported: latest,
        });
    }
    for migration in migrations.iter().filter(|m| m.version > current) {
        // IMMEDIATE: take the write lock up front so a concurrent opener waits instead of
        // failing half-way through the script.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(migration.sql)?;
        tx.execute(
            "INSERT INTO migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            (migration.version, migration.name, crate::now_millis()),
        )?;
        tx.commit()?;
        tracing::debug!(
            version = migration.version,
            name = migration.name,
            "state migration applied"
        );
    }
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(conn: &Connection) -> Vec<u32> {
        let mut stmt = conn
            .prepare("SELECT version FROM migrations ORDER BY version")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn versions_are_gap_free_and_ordered() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version as usize, i + 1, "{}", m.name);
        }
    }

    #[test]
    fn applies_in_order_and_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(apply(&mut conn).unwrap(), latest_version());
        let expected: Vec<u32> = MIGRATIONS.iter().map(|m| m.version).collect();
        assert_eq!(versions(&conn), expected);
        let applied_at: i64 = conn
            .query_row(
                "SELECT applied_at FROM migrations WHERE version = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();

        // A second open applies nothing and rewrites nothing.
        assert_eq!(apply(&mut conn).unwrap(), latest_version());
        assert_eq!(versions(&conn), expected);
        let again: i64 = conn
            .query_row(
                "SELECT applied_at FROM migrations WHERE version = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(applied_at, again);
    }

    #[test]
    fn resumes_a_partially_migrated_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(CREATE_MIGRATIONS_TABLE).unwrap();
        // A database that stopped before the first migration: nothing recorded.
        assert!(versions(&conn).is_empty());
        apply(&mut conn).unwrap();
        assert_eq!(versions(&conn).len(), MIGRATIONS.len());
    }

    #[test]
    fn a_failing_migration_rolls_back_and_records_nothing() {
        // The real first migration, then one whose second statement is invalid: the first
        // statement of the broken one runs before the failure and must not survive it.
        let broken = [
            Migration {
                version: 1,
                name: "init",
                sql: MIGRATIONS[0].sql,
            },
            Migration {
                version: 2,
                name: "broken",
                sql: "CREATE TABLE half_applied (a); CREATE TABLE half_applied (a);",
            },
        ];
        let mut conn = Connection::open_in_memory().unwrap();
        let err = apply_all(&mut conn, &broken).unwrap_err();
        assert!(matches!(err, StateFailure::Sqlite(_)), "{err:?}");

        assert_eq!(
            versions(&conn),
            [1],
            "only the migration that succeeded is recorded"
        );
        let leftover: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'half_applied'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftover, 0, "the failed migration left nothing behind");
        // The connection is usable and the earlier migration is intact.
        assert_eq!(apply(&mut conn).unwrap(), latest_version());
    }

    #[test]
    fn a_newer_database_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO migrations (version, name, applied_at) VALUES (?1, 'future', 0)",
            [latest_version() + 1],
        )
        .unwrap();
        let err = apply(&mut conn).unwrap_err();
        assert!(matches!(err, StateFailure::Newer { .. }), "{err:?}");
    }
}
