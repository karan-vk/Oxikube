//! [`StatePort`]: durable local state: a key-value store, typed tables and the audit log.
//!
//! # Adapter
//!
//! Implemented by `oxikube_state_sqlite` on rusqlite with in-repo migrations (ADR 0010).
//! No `rusqlite` type crosses this port; values are `serde_json::Value`.
//!
//! # Never a secret
//!
//! `StatePort` is on disk. **It must never be given a secret** (non-negotiable 5,
//! ADR 0010): tokens, kubeconfig credentials and decoded Secret data go to
//! `SecretStorePort`. [`AuditRecord`] carries no request bodies for the same reason.
//!
//! # Shape
//!
//! * `kv_*`: small free-form settings-like values (recent clusters, window layout).
//! * `table_*`: rows in a named [`StateTable`] keyed by [`StateKey`] (favourites,
//!   port-forward presets, agent threads).
//! * `append_audit` / `query_audit`: the append-only audit log, written by
//!   `MutationGuard` in batches.
//!
//! The trait stays object-safe by speaking `serde_json::Value`; [`StatePortExt`]
//! adds typed `get`/`put` helpers for any `StatePort`, including `dyn StatePort`.
//! All calls run off the UI thread.

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::audit::AuditRecord;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

const MAX_NAME_BYTES: usize = 256;

fn validate_name(what: &str, name: &str, allow_extra: bool) -> OxiResult<()> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-')
                || (allow_extra && matches!(c, '.' | '/' | ':'))
        });
    if ok {
        Ok(())
    } else {
        Err(OxiError::validation(format!(
            "state {what} must be 1-{MAX_NAME_BYTES} bytes of [A-Za-z0-9_-]{}",
            if allow_extra {
                " plus '.', '/' and ':'"
            } else {
                ""
            }
        )))
    }
}

/// A key in the kv store or in a table. Restricted to `[A-Za-z0-9_-]` plus `.`, `/`
/// and `:` (so ids and paths work) and at most 256 bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateKey(String);

impl StateKey {
    /// Validates and wraps a key.
    pub fn new(key: impl Into<String>) -> OxiResult<Self> {
        let key = key.into();
        validate_name("key", &key, true)?;
        Ok(Self(key))
    }

    /// The key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The name of a typed table. Restricted to `[A-Za-z0-9_-]`, at most 256 bytes, so
/// an adapter can map it to storage safely.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateTable(String);

impl StateTable {
    /// Validates and wraps a table name.
    pub fn new(name: impl Into<String>) -> OxiResult<Self> {
        let name = name.into();
        validate_name("table", &name, false)?;
        Ok(Self(name))
    }

    /// The table name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Filter for [`StatePort::query_audit`]. All set fields must match; the default
/// matches everything (up to `limit`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditQuery {
    /// Only records for this cluster.
    pub cluster: Option<ClusterId>,
    /// Only records at or after this time.
    pub since: Option<Timestamp>,
    /// Only records before this time.
    pub until: Option<Timestamp>,
    /// Only records of this command id.
    pub cmd: Option<String>,
    /// Maximum number of records returned.
    pub limit: usize,
}

impl Default for AuditQuery {
    fn default() -> Self {
        Self {
            cluster: None,
            since: None,
            until: None,
            cmd: None,
            limit: 200,
        }
    }
}

/// Durable local state.
///
/// # Effects
///
/// Mutating on the local SQLite database only (`*_set`, `*_put`, `*_delete`,
/// [`append_audit`](Self::append_audit)); never a cluster mutation.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`Validation`](oxikube_domain::ErrorKind::Validation) for an unknown table or malformed key,
/// [`Conflict`](oxikube_domain::ErrorKind::Conflict) for a failed migration or a concurrent
/// writer, [`Internal`](oxikube_domain::ErrorKind::Internal) for database I/O failures. A
/// missing key is `Ok(None)` or `Ok(false)`.
#[async_trait]
pub trait StatePort: Send + Sync {
    /// The value under `key`, or `None`.
    async fn kv_get(&self, key: &StateKey) -> OxiResult<Option<Value>>;

    /// Stores `value` under `key`, replacing any existing value.
    async fn kv_set(&self, key: &StateKey, value: Value) -> OxiResult<()>;

    /// Removes `key`. Returns whether it existed.
    async fn kv_delete(&self, key: &StateKey) -> OxiResult<bool>;

    /// Every kv entry whose key starts with `prefix`, ordered by key.
    async fn kv_list(&self, prefix: &str) -> OxiResult<Vec<(StateKey, Value)>>;

    /// The row under `key` in `table`, or `None`.
    async fn table_get(&self, table: &StateTable, key: &StateKey) -> OxiResult<Option<Value>>;

    /// Inserts or replaces the row under `key` in `table`.
    async fn table_put(&self, table: &StateTable, key: &StateKey, row: Value) -> OxiResult<()>;

    /// Removes the row under `key`. Returns whether it existed.
    async fn table_delete(&self, table: &StateTable, key: &StateKey) -> OxiResult<bool>;

    /// Up to `limit` rows of `table` (all when `None`), ordered by key.
    async fn table_list(
        &self,
        table: &StateTable,
        limit: Option<usize>,
    ) -> OxiResult<Vec<(StateKey, Value)>>;

    /// Appends `records` to the audit log in one batch. The log is append-only.
    async fn append_audit(&self, records: &[AuditRecord]) -> OxiResult<()>;

    /// Records matching `query`, newest first.
    async fn query_audit(&self, query: &AuditQuery) -> OxiResult<Vec<AuditRecord>>;
}

/// Typed helpers over [`StatePort`]'s JSON values. Implemented for every
/// `StatePort`, including `dyn StatePort`. Never store secrets, even typed ones.
///
/// # Effects
///
/// Same effects as the [`StatePort`] methods it wraps.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// Those of [`StatePort`], plus [`Validation`](oxikube_domain::ErrorKind::Validation) when a
/// stored value does not match the requested type.
#[async_trait]
pub trait StatePortExt: StatePort {
    /// Reads and deserialises the kv value under `key`. A value of the wrong shape
    /// is a `Validation` error.
    async fn kv_get_as<T: DeserializeOwned + Send>(&self, key: &StateKey) -> OxiResult<Option<T>> {
        match self.kv_get(key).await? {
            Some(value) => decode(value).map(Some),
            None => Ok(None),
        }
    }

    /// Serialises and stores `value` under `key`.
    async fn kv_set_as<T: Serialize + Send + Sync>(
        &self,
        key: &StateKey,
        value: &T,
    ) -> OxiResult<()> {
        self.kv_set(key, encode(value)?).await
    }

    /// Reads and deserialises a table row.
    async fn table_get_as<T: DeserializeOwned + Send>(
        &self,
        table: &StateTable,
        key: &StateKey,
    ) -> OxiResult<Option<T>> {
        match self.table_get(table, key).await? {
            Some(value) => decode(value).map(Some),
            None => Ok(None),
        }
    }

    /// Serialises and stores a table row.
    async fn table_put_as<T: Serialize + Send + Sync>(
        &self,
        table: &StateTable,
        key: &StateKey,
        row: &T,
    ) -> OxiResult<()> {
        self.table_put(table, key, encode(row)?).await
    }
}

impl<P: StatePort + ?Sized> StatePortExt for P {}

fn decode<T: DeserializeOwned>(value: Value) -> OxiResult<T> {
    serde_json::from_value(value)
        .map_err(|e| OxiError::validation("stored state has an unexpected shape").with_source(e))
}

fn encode<T: Serialize>(value: &T) -> OxiResult<Value> {
    serde_json::to_value(value)
        .map_err(|e| OxiError::validation("value cannot be stored as state").with_source(e))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use serde::Deserialize;

    use super::*;

    #[derive(Default)]
    struct MemState {
        kv: Mutex<BTreeMap<String, Value>>,
        tables: Mutex<BTreeMap<(String, String), Value>>,
    }

    #[async_trait]
    impl StatePort for MemState {
        async fn kv_get(&self, key: &StateKey) -> OxiResult<Option<Value>> {
            Ok(self.kv.lock().unwrap().get(key.as_str()).cloned())
        }
        async fn kv_set(&self, key: &StateKey, value: Value) -> OxiResult<()> {
            self.kv.lock().unwrap().insert(key.as_str().into(), value);
            Ok(())
        }
        async fn kv_delete(&self, key: &StateKey) -> OxiResult<bool> {
            Ok(self.kv.lock().unwrap().remove(key.as_str()).is_some())
        }
        async fn kv_list(&self, prefix: &str) -> OxiResult<Vec<(StateKey, Value)>> {
            let kv = self.kv.lock().unwrap();
            kv.iter()
                .filter(|(k, _)| k.starts_with(prefix))
                .map(|(k, v)| Ok((StateKey::new(k.clone())?, v.clone())))
                .collect()
        }
        async fn table_get(&self, table: &StateTable, key: &StateKey) -> OxiResult<Option<Value>> {
            let k = (table.as_str().to_owned(), key.as_str().to_owned());
            Ok(self.tables.lock().unwrap().get(&k).cloned())
        }
        async fn table_put(&self, table: &StateTable, key: &StateKey, row: Value) -> OxiResult<()> {
            let k = (table.as_str().to_owned(), key.as_str().to_owned());
            self.tables.lock().unwrap().insert(k, row);
            Ok(())
        }
        async fn table_delete(&self, table: &StateTable, key: &StateKey) -> OxiResult<bool> {
            let k = (table.as_str().to_owned(), key.as_str().to_owned());
            Ok(self.tables.lock().unwrap().remove(&k).is_some())
        }
        async fn table_list(
            &self,
            table: &StateTable,
            limit: Option<usize>,
        ) -> OxiResult<Vec<(StateKey, Value)>> {
            let rows = self.tables.lock().unwrap();
            rows.iter()
                .filter(|((t, _), _)| t == table.as_str())
                .take(limit.unwrap_or(usize::MAX))
                .map(|((_, k), v)| Ok((StateKey::new(k.clone())?, v.clone())))
                .collect()
        }
        async fn append_audit(&self, _records: &[AuditRecord]) -> OxiResult<()> {
            Ok(())
        }
        async fn query_audit(&self, _query: &AuditQuery) -> OxiResult<Vec<AuditRecord>> {
            Ok(Vec::new())
        }
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Layout {
        sidebar_width: u32,
    }

    #[test]
    fn typed_helpers_round_trip_through_a_dyn_port() {
        let state: Arc<dyn StatePort> = Arc::new(MemState::default());
        let key = StateKey::new("window/layout").unwrap();
        let table = StateTable::new("favourites").unwrap();
        futures::executor::block_on(async {
            assert_eq!(state.kv_get_as::<Layout>(&key).await.unwrap(), None);
            state
                .kv_set_as(&key, &Layout { sidebar_width: 240 })
                .await
                .unwrap();
            assert_eq!(
                state.kv_get_as::<Layout>(&key).await.unwrap(),
                Some(Layout { sidebar_width: 240 })
            );
            state
                .table_put_as(&table, &key, &Layout { sidebar_width: 1 })
                .await
                .unwrap();
            assert_eq!(
                state.table_get_as::<Layout>(&table, &key).await.unwrap(),
                Some(Layout { sidebar_width: 1 })
            );
            // A value of the wrong shape is a validation error, not a panic.
            state.kv_set(&key, Value::from("nope")).await.unwrap();
            let err = state.kv_get_as::<Layout>(&key).await.unwrap_err();
            assert_eq!(err.kind(), oxikube_domain::ErrorKind::Validation);
        });
    }

    #[test]
    fn names_are_validated() {
        assert!(StateKey::new("recent:cluster/abc.1").is_ok());
        assert!(StateTable::new("port_forwards").is_ok());
        for bad in ["", "has space", "semi;colon", "quote'"] {
            assert!(StateKey::new(bad).is_err(), "{bad:?}");
            assert!(StateTable::new(bad).is_err(), "{bad:?}");
        }
        assert!(StateTable::new("a/b").is_err());
        assert!(StateKey::new("k".repeat(257)).is_err());
    }
}
