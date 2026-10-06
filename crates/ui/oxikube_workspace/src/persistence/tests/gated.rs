//! A `StatePort` whose first `table_get` waits for a gate, to hold a restore in flight.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::channel::oneshot;
use oxikube_domain::{OxiResult, audit::AuditRecord};
use oxikube_ports::{AuditQuery, StateKey, StatePort, StateTable};
use oxikube_testkit::fakes::FakeStatePort;
use serde_json::Value;

pub struct GatedState {
    pub inner: Arc<FakeStatePort>,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
}

impl GatedState {
    pub fn new(inner: Arc<FakeStatePort>) -> (Arc<Self>, oneshot::Sender<()>) {
        let (open, gate) = oneshot::channel();
        (
            Arc::new(Self {
                inner,
                gate: Mutex::new(Some(gate)),
            }),
            open,
        )
    }
}

#[async_trait]
impl StatePort for GatedState {
    async fn kv_get(&self, key: &StateKey) -> OxiResult<Option<Value>> {
        self.inner.kv_get(key).await
    }
    async fn kv_set(&self, key: &StateKey, value: Value) -> OxiResult<()> {
        self.inner.kv_set(key, value).await
    }
    async fn kv_delete(&self, key: &StateKey) -> OxiResult<bool> {
        self.inner.kv_delete(key).await
    }
    async fn kv_list(&self, prefix: &str) -> OxiResult<Vec<(StateKey, Value)>> {
        self.inner.kv_list(prefix).await
    }
    async fn table_get(&self, table: &StateTable, key: &StateKey) -> OxiResult<Option<Value>> {
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.await;
        }
        self.inner.table_get(table, key).await
    }
    async fn table_put(&self, table: &StateTable, key: &StateKey, row: Value) -> OxiResult<()> {
        self.inner.table_put(table, key, row).await
    }
    async fn table_delete(&self, table: &StateTable, key: &StateKey) -> OxiResult<bool> {
        self.inner.table_delete(table, key).await
    }
    async fn table_list(
        &self,
        table: &StateTable,
        limit: Option<usize>,
    ) -> OxiResult<Vec<(StateKey, Value)>> {
        self.inner.table_list(table, limit).await
    }
    async fn append_audit(&self, records: &[AuditRecord]) -> OxiResult<()> {
        self.inner.append_audit(records).await
    }
    async fn query_audit(&self, query: &AuditQuery) -> OxiResult<Vec<AuditRecord>> {
        self.inner.query_audit(query).await
    }
}
