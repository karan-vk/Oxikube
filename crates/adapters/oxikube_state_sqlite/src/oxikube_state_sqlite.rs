//! `oxikube_state_sqlite` — layer: `adapters`.
//!
//! StatePort adapter on rusqlite (bundled): migrations, workspace layout, per-cluster state, favourites, audit log, caches.
//!
//! [`SqliteState`] implements `oxikube_ports::StatePort` (ADR 0010) on one SQLite file:
//!
//! - [`migrations`]: ordered, embedded SQL scripts applied in a transaction each on open, recorded
//!   in a `migrations` table; idempotent.
//! - the kv store and the typed tables share one `kv (namespace, key, value JSON, updated_at)`
//!   table (the kv store is namespace `''`, a typed table its own name); the audit log has its own
//!   append-only table with the columns `AuditQuery` filters on.
//! - every call runs on one dedicated thread that owns the connection, so nothing blocks the UI
//!   thread, not even the open.
//! - a damaged file is moved aside to `<name>.corrupt-<timestamp>` and replaced by a fresh
//!   database ([`Recovery`]); settings and keymap files are never touched.
//!
//! Tests use a temp directory: `SqliteState::open(dir.path().join("state.db"))`.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

mod audit;
mod error;
mod kv;
pub mod migrations;
mod open;
mod state;
mod worker;

#[cfg(test)]
mod tests;

pub use error::StateFailure;
pub use open::Recovery;
pub use state::SqliteState;

/// Milliseconds since the Unix epoch (`kv.updated_at`, `migrations.applied_at`).
pub(crate) fn now_millis() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}
