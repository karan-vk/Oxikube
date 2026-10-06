//! Failures of the SQLite state store and their mapping onto `OxiError` (the table in
//! `docs/ARCHITECTURE.md`).

use oxikube_domain::OxiError;
use rusqlite::ErrorCode;

/// Why a database operation failed, before it becomes an `OxiError`.
#[derive(Debug)]
pub enum StateFailure {
    /// SQLite reported the file is damaged (not a database, malformed image, failed check). The
    /// opener moves such a file aside and starts fresh.
    Corrupt(String),
    /// The database was written by a newer Oxikube: refused, never moved aside or reset.
    Newer {
        /// The schema version recorded in the file.
        found: u32,
        /// The newest version this build knows.
        supported: u32,
    },
    /// Anything else SQLite or the OS said.
    Sqlite(rusqlite::Error),
    /// Filesystem trouble around the database file.
    Io(std::io::Error),
}

impl From<rusqlite::Error> for StateFailure {
    fn from(e: rusqlite::Error) -> Self {
        match e.sqlite_error_code() {
            Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) => {
                Self::Corrupt(e.to_string())
            }
            _ => Self::Sqlite(e),
        }
    }
}

impl From<std::io::Error> for StateFailure {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<StateFailure> for OxiError {
    fn from(f: StateFailure) -> Self {
        match f {
            StateFailure::Corrupt(why) => {
                OxiError::internal("the state database is corrupt").with_source(Message(why))
            }
            StateFailure::Newer { found, supported } => OxiError::conflict(format!(
                "the state database is at schema version {found}, newer than this build's {supported}"
            )),
            StateFailure::Sqlite(e) => match e.sqlite_error_code() {
                // A concurrent writer holds the lock past the busy timeout.
                Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
                    OxiError::conflict("the state database is busy")
                        .with_source(e)
                        .with_retryable(true)
                }
                _ => OxiError::internal("state database error").with_source(e),
            },
            StateFailure::Io(e) => OxiError::internal("state database file error").with_source(e),
        }
    }
}

/// Maps a rusqlite error straight to an `OxiError`.
pub(crate) fn oxi(e: rusqlite::Error) -> OxiError {
    StateFailure::from(e).into()
}

#[derive(Debug)]
struct Message(String);

impl std::fmt::Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Message {}
