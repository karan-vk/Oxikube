//! [`SqliteState`]: the `StatePort` implementation.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use oxikube_domain::{OxiError, OxiResult, audit::AuditRecord};
use oxikube_ports::{AuditQuery, StateKey, StatePort, StateTable};
use serde_json::Value;

use crate::{
    audit,
    kv::{self, KV_NAMESPACE},
    open::{self, Recovery},
    worker::Worker,
};

/// Durable local state in one SQLite file (ADR 0010).
///
/// Every call runs on a dedicated thread that owns the connection; the returned futures only wait
/// for a oneshot, so they are safe to await from the UI thread's executor. Dropping the store
/// finishes the queued writes and closes the database.
///
/// Never give it a secret (non-negotiable 5): it is a plain file.
pub struct SqliteState {
    worker: Worker,
    path: PathBuf,
    recovery: Option<Recovery>,
}

impl std::fmt::Debug for SqliteState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteState")
            .field("path", &self.path)
            .field("recovered", &self.recovery.is_some())
            .finish_non_exhaustive()
    }
}

impl SqliteState {
    /// Opens (creating when absent) the database at `path` and migrates it. The open, the quick
    /// integrity check and the migrations run on the store's own thread, so awaiting this does
    /// not block the caller's thread.
    ///
    /// A damaged file is moved aside to `<path>.corrupt-<timestamp>` and replaced by a fresh
    /// database; see [`SqliteState::recovery`]. A database written by a newer build is refused
    /// with a `Conflict` error and left alone.
    ///
    /// # Errors
    ///
    /// `Conflict` for a newer or busy database, `Internal` for file or SQLite failures.
    pub async fn open(path: impl Into<PathBuf>) -> OxiResult<Self> {
        let path = path.into();
        let thread_path = path.clone();
        let (worker, ready) =
            Worker::spawn(move || open::open(&thread_path).map_err(OxiError::from))?;
        let recovery = ready
            .await
            .map_err(|_| OxiError::internal("the state database thread stopped while opening"))??;
        Ok(Self {
            worker,
            path,
            recovery,
        })
    }

    /// Where the database file is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Set when [`SqliteState::open`] found the file damaged, moved it aside and started a fresh
    /// database: the caller can tell the user their saved layout was reset. `None` for a normal
    /// open.
    pub fn recovery(&self) -> Option<&Recovery> {
        self.recovery.as_ref()
    }
}

#[async_trait]
impl StatePort for SqliteState {
    async fn kv_get(&self, key: &StateKey) -> OxiResult<Option<Value>> {
        let key = key.as_str().to_owned();
        self.worker
            .run(move |conn| kv::get(conn, KV_NAMESPACE, &key))
            .await
    }

    async fn kv_set(&self, key: &StateKey, value: Value) -> OxiResult<()> {
        let key = key.as_str().to_owned();
        self.worker
            .run(move |conn| kv::set(conn, KV_NAMESPACE, &key, &value))
            .await
    }

    async fn kv_delete(&self, key: &StateKey) -> OxiResult<bool> {
        let key = key.as_str().to_owned();
        self.worker
            .run(move |conn| kv::delete(conn, KV_NAMESPACE, &key))
            .await
    }

    async fn kv_list(&self, prefix: &str) -> OxiResult<Vec<(StateKey, Value)>> {
        let prefix = prefix.to_owned();
        let rows = self
            .worker
            .run(move |conn| kv::list(conn, KV_NAMESPACE, &prefix, None))
            .await?;
        keyed(rows)
    }

    async fn table_get(&self, table: &StateTable, key: &StateKey) -> OxiResult<Option<Value>> {
        let (table, key) = (table.as_str().to_owned(), key.as_str().to_owned());
        self.worker
            .run(move |conn| kv::get(conn, &table, &key))
            .await
    }

    async fn table_put(&self, table: &StateTable, key: &StateKey, row: Value) -> OxiResult<()> {
        let (table, key) = (table.as_str().to_owned(), key.as_str().to_owned());
        self.worker
            .run(move |conn| kv::set(conn, &table, &key, &row))
            .await
    }

    async fn table_delete(&self, table: &StateTable, key: &StateKey) -> OxiResult<bool> {
        let (table, key) = (table.as_str().to_owned(), key.as_str().to_owned());
        self.worker
            .run(move |conn| kv::delete(conn, &table, &key))
            .await
    }

    async fn table_list(
        &self,
        table: &StateTable,
        limit: Option<usize>,
    ) -> OxiResult<Vec<(StateKey, Value)>> {
        let table = table.as_str().to_owned();
        let rows = self
            .worker
            .run(move |conn| kv::list(conn, &table, "", limit))
            .await?;
        keyed(rows)
    }

    async fn append_audit(&self, records: &[AuditRecord]) -> OxiResult<()> {
        let records = records.to_vec();
        self.worker
            .run(move |conn| audit::append(conn, &records))
            .await
    }

    async fn query_audit(&self, query: &AuditQuery) -> OxiResult<Vec<AuditRecord>> {
        let query = query.clone();
        self.worker
            .run(move |conn| audit::query(conn, &query))
            .await
    }
}

fn keyed(rows: Vec<(String, Value)>) -> OxiResult<Vec<(StateKey, Value)>> {
    rows.into_iter()
        .map(|(key, value)| Ok((StateKey::new(key)?, value)))
        .collect()
}
