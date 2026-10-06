//! The state database, opened in the background.
//!
//! Opening SQLite (create, `quick_check`, migrations) takes a few milliseconds on a warm disk and
//! more on a cold one, and the first frame must not wait for it (docs/PERFORMANCE.md, cold
//! start). [`LazyState`] is the `StatePort` that `AppState` holds: it exists at once, starts the
//! open on the background executor ([`LazyState::start`]) and every call awaits that open first.
//! The adapter already runs each call on its own thread, so nothing here touches the UI thread;
//! a caller that reaches the port before the open finishes (the layout restore) just waits on
//! its own task.
//!
//! If the open fails, every call returns the same error (kind kept, message prefixed with
//! "state database unavailable") and the app runs without persistence: views already treat a
//! failed `StatePort` as "use the defaults".

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared};
use gpui::{App, AppContext as _, Task};
use oxikube_domain::OxiError;
use oxikube_domain::OxiResult;
use oxikube_domain::audit::AuditRecord;
use oxikube_ports::{AuditQuery, StateKey, StatePort, StateTable};
use oxikube_state_sqlite::SqliteState;
use serde_json::Value;

type Opened = Result<Arc<SqliteState>, Arc<OxiError>>;

/// A `StatePort` over a SQLite database that is still opening (or failed to). See the module docs.
#[derive(Clone)]
pub struct LazyState {
    opened: Shared<BoxFuture<'static, Opened>>,
}

impl LazyState {
    /// Prepares the open of the database at `path`. Nothing runs until [`LazyState::start`] (or
    /// the first port call) polls it.
    pub fn new(path: PathBuf) -> Self {
        let opened = async move {
            SqliteState::open(path)
                .await
                .map(Arc::new)
                .map_err(Arc::new)
        }
        .boxed()
        .shared();
        Self { opened }
    }

    /// Starts the open on the background executor and logs how it went. The task needs no owner
    /// (the open is also driven by whichever port call comes first); drop it to stop only the
    /// logging.
    pub fn start(&self, cx: &App) -> Task<()> {
        let opened = self.opened.clone();
        cx.background_spawn(async move {
            let started = Instant::now();
            match opened.await {
                Ok(db) => {
                    tracing::info!(
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        path = %db.path().display(),
                        "state database open"
                    );
                    if let Some(recovery) = db.recovery() {
                        tracing::warn!(
                            moved_to = %recovery.moved_to.display(),
                            "the state database was damaged and has been reset"
                        );
                    }
                }
                Err(err) => {
                    tracing::error!(%err, "state database could not be opened; running without saved state");
                }
            }
        })
    }

    async fn ready(&self) -> OxiResult<Arc<SqliteState>> {
        self.opened.clone().await.map_err(|err| {
            OxiError::new(
                err.kind(),
                format!("state database unavailable: {}", err.message()),
            )
        })
    }
}

#[async_trait]
impl StatePort for LazyState {
    async fn kv_get(&self, key: &StateKey) -> OxiResult<Option<Value>> {
        self.ready().await?.kv_get(key).await
    }

    async fn kv_set(&self, key: &StateKey, value: Value) -> OxiResult<()> {
        self.ready().await?.kv_set(key, value).await
    }

    async fn kv_delete(&self, key: &StateKey) -> OxiResult<bool> {
        self.ready().await?.kv_delete(key).await
    }

    async fn kv_list(&self, prefix: &str) -> OxiResult<Vec<(StateKey, Value)>> {
        self.ready().await?.kv_list(prefix).await
    }

    async fn table_get(&self, table: &StateTable, key: &StateKey) -> OxiResult<Option<Value>> {
        self.ready().await?.table_get(table, key).await
    }

    async fn table_put(&self, table: &StateTable, key: &StateKey, row: Value) -> OxiResult<()> {
        self.ready().await?.table_put(table, key, row).await
    }

    async fn table_delete(&self, table: &StateTable, key: &StateKey) -> OxiResult<bool> {
        self.ready().await?.table_delete(table, key).await
    }

    async fn table_list(
        &self,
        table: &StateTable,
        limit: Option<usize>,
    ) -> OxiResult<Vec<(StateKey, Value)>> {
        self.ready().await?.table_list(table, limit).await
    }

    async fn append_audit(&self, records: &[AuditRecord]) -> OxiResult<()> {
        self.ready().await?.append_audit(records).await
    }

    async fn query_audit(&self, query: &AuditQuery) -> OxiResult<Vec<AuditRecord>> {
        self.ready().await?.query_audit(query).await
    }
}
