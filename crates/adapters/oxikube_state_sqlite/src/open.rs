//! Opening the database: pragmas, the quick integrity check, migrations, and the corrupt-state
//! fallback.
//!
//! A database that is damaged (random bytes, a truncated page, a failed check) is moved aside to
//! `<name>.corrupt-<timestamp>` (with its `-wal` and `-shm` files) and a fresh one is created at
//! the original path; the move is logged and reported through [`Recovery`]. Only that one file
//! family is touched: the settings and keymap files live elsewhere and are never read or moved
//! here (ADR 0010). Anything that is not damage (a locked file, a missing directory permission,
//! a database from a newer build) is returned as an error and nothing is moved.

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use rusqlite::Connection;

use crate::{error::StateFailure, migrations};

/// A damaged database was set aside and replaced by a fresh one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    /// Where the damaged file now is.
    pub moved_to: PathBuf,
    /// What SQLite reported (no values from the file).
    pub reason: String,
}

/// Opens (creating when absent) the database at `path`, migrated to the latest schema.
///
/// Blocking: runs on the store's worker thread, never on the UI thread.
pub(crate) fn open(path: &Path) -> Result<(Connection, Option<Recovery>), StateFailure> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    match open_checked(path) {
        Ok(conn) => Ok((conn, None)),
        Err(StateFailure::Corrupt(reason)) => {
            let moved_to = move_aside(path)?;
            tracing::warn!(
                path = %path.display(),
                moved_to = %moved_to.display(),
                %reason,
                "state database is corrupt: moved aside, starting with a fresh one"
            );
            let conn = open_checked(path)?;
            Ok((conn, Some(Recovery { moved_to, reason })))
        }
        Err(other) => Err(other),
    }
}

fn open_checked(path: &Path) -> Result<Connection, StateFailure> {
    let mut conn = Connection::open(path)?;
    // `quick_check` (not `integrity_check`): it skips the index/content cross-checks, so it is
    // cheap enough for every launch, and a random-bytes or truncated file fails it at once.
    let verdict: String = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
    if verdict != "ok" {
        return Err(StateFailure::Corrupt(format!("quick_check: {verdict}")));
    }
    // WAL: readers never block the single writer and a write is one append, which keeps the
    // debounced layout save cheap. NORMAL is durable enough for UI state under WAL.
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    migrations::apply(&mut conn)?;
    Ok(conn)
}

/// Renames the database file family to `<name>.corrupt-<timestamp>[-n]`, returning the new path
/// of the main file.
fn move_aside(path: &Path) -> Result<PathBuf, StateFailure> {
    let stamp = Timestamp::now().strftime("%Y%m%dT%H%M%SZ").to_string();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state.db".to_owned());
    let mut target = path.with_file_name(format!("{name}.corrupt-{stamp}"));
    let mut n = 1;
    while target.exists() {
        target = path.with_file_name(format!("{name}.corrupt-{stamp}-{n}"));
        n += 1;
    }
    std::fs::rename(path, &target)?;
    // The sidecar files belong to the damaged database: a stale WAL applied to a fresh file
    // would corrupt it too.
    for suffix in ["-wal", "-shm"] {
        let sidecar = sidecar(path, suffix);
        if sidecar.exists() {
            let mut dest = target.clone().into_os_string();
            dest.push(suffix);
            std::fs::rename(&sidecar, PathBuf::from(dest))?;
        }
    }
    Ok(target)
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(suffix);
    PathBuf::from(p)
}
