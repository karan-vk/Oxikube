//! [`DiscoveryCache`]: the last listing of every scope, kept in the state db with its time.
//!
//! Two `StatePort` tables (typed tables are rows of the generic `kv` table, so no schema
//! migration is needed): `cloud_scopes` holds one row per scope (its clusters and when they were
//! listed), keyed by the scope's [`key`](CloudScope::key); `cloud_providers` holds one row per
//! provider (the last detection and the scope list the CLI reported), keyed by its id. Only
//! [`DiscoveredCluster`] data is stored: names, locations, endpoints, public CA data, never a
//! token. A cache that cannot be read or written is logged and treated as empty: discovery
//! still works, it only runs more often.

use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    ClockPort, CloudDetection, CloudProvider, CloudScope, DiscoveredCluster, StateKey, StatePort,
    StatePortExt as _, StateTable,
};
use serde::{Deserialize, Serialize};

/// The table of per-scope listings.
const SCOPES_TABLE: &str = "cloud_scopes";
/// The table of per-provider detections and scope lists.
const PROVIDERS_TABLE: &str = "cloud_providers";

/// One scope's last listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ScopeRow {
    pub scope: CloudScope,
    /// Unix milliseconds.
    pub fetched_at: i64,
    pub clusters: Vec<DiscoveredCluster>,
}

/// One provider's last detection and scope list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProviderRow {
    pub detection: CloudDetection,
    /// The scopes the CLI reported last; `None` until a run listed them.
    pub scopes: Option<Vec<CloudScope>>,
    /// Unix milliseconds.
    pub fetched_at: i64,
}

/// Everything cached for one provider.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub provider: Option<ProviderRow>,
    pub scopes: Vec<ScopeRow>,
}

impl Snapshot {
    /// Whether nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.provider.is_none() && self.scopes.is_empty()
    }

    /// The scopes a run without an explicit list covers: the last reported list, else every
    /// cached scope.
    pub fn known_scopes(&self) -> Vec<CloudScope> {
        match self.provider.as_ref().and_then(|p| p.scopes.clone()) {
            Some(scopes) => scopes,
            None => self.scopes.iter().map(|row| row.scope.clone()).collect(),
        }
    }

    /// The cached rows of `scopes`, in that order (missing ones skipped).
    pub fn rows_of<'snapshot>(&'snapshot self, scopes: &[CloudScope]) -> Vec<&'snapshot ScopeRow> {
        scopes
            .iter()
            .filter_map(|scope| self.scopes.iter().find(|row| row.scope == *scope))
            .collect()
    }

    /// Whether the detection and every one of `scopes` (or of the known scopes when `None`) were
    /// listed within `ttl` of `now`. A scope list never reported is not fresh.
    pub fn is_fresh(&self, scopes: Option<&[CloudScope]>, ttl: Duration, now: Timestamp) -> bool {
        let fresh = |at: i64| is_within(at, ttl, now);
        let Some(provider) = &self.provider else {
            return false;
        };
        if !fresh(provider.fetched_at) {
            return false;
        }
        let wanted = match scopes {
            Some(scopes) => scopes.to_vec(),
            None if provider.scopes.is_none() => return false,
            None => self.known_scopes(),
        };
        wanted.iter().all(|scope| {
            self.scopes
                .iter()
                .any(|row| row.scope == *scope && fresh(row.fetched_at))
        })
    }
}

/// Whether `at` (unix ms) lies in `[now - ttl, now]`. A time in the future (the clock moved
/// back) is stale, so a wrong clock costs a refresh, never a stale list kept forever.
fn is_within(at: i64, ttl: Duration, now: Timestamp) -> bool {
    let age = now.as_millisecond().saturating_sub(at);
    let ttl = i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX);
    (0..ttl).contains(&age)
}

/// The cache over `StatePort`. Cheap to clone.
#[derive(Clone)]
pub(super) struct DiscoveryCache {
    state: Arc<dyn StatePort>,
    clock: Arc<dyn ClockPort>,
}

impl DiscoveryCache {
    pub fn new(state: Arc<dyn StatePort>, clock: Arc<dyn ClockPort>) -> Self {
        Self { state, clock }
    }

    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// Everything cached for `provider`. Unreadable rows are skipped (and logged).
    pub async fn load(&self, provider: CloudProvider) -> Snapshot {
        match self.try_load(provider).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(%error, %provider, "the cloud discovery cache could not be read");
                Snapshot::default()
            }
        }
    }

    async fn try_load(&self, provider: CloudProvider) -> OxiResult<Snapshot> {
        let provider_row = self
            .state
            .table_get_as::<ProviderRow>(&providers_table()?, &state_key(provider.id())?)
            .await?;
        let rows = self.state.table_list(&scopes_table()?, None).await?;
        let scopes = rows
            .into_iter()
            .filter_map(|(key, value)| match serde_json::from_value::<ScopeRow>(value) {
                Ok(row) => Some(row),
                Err(error) => {
                    tracing::warn!(%error, key = key.as_str(), "skipping an unreadable cloud cache row");
                    None
                }
            })
            .filter(|row| row.scope.provider() == provider)
            .collect();
        Ok(Snapshot {
            provider: provider_row,
            scopes,
        })
    }

    /// Stores the listing of one scope, stamped now.
    pub async fn put_scope(&self, scope: &CloudScope, clusters: &[DiscoveredCluster]) {
        let row = ScopeRow {
            scope: scope.clone(),
            fetched_at: self.now().as_millisecond(),
            clusters: clusters.to_vec(),
        };
        let result = async {
            let key = state_key(&scope.key())?;
            self.state.table_put_as(&scopes_table()?, &key, &row).await
        };
        log_failure(result.await, "store a scope listing");
    }

    /// Stores the provider's detection, and the scope list when the CLI reported one.
    pub async fn put_provider(&self, detection: &CloudDetection, scopes: Option<&[CloudScope]>) {
        let previous = self.load(detection.provider).await.provider;
        let row = ProviderRow {
            detection: detection.clone(),
            scopes: scopes
                .map(<[CloudScope]>::to_vec)
                .or_else(|| previous.and_then(|p| p.scopes)),
            fetched_at: self.now().as_millisecond(),
        };
        let result = async {
            let key = state_key(detection.provider.id())?;
            self.state
                .table_put_as(&providers_table()?, &key, &row)
                .await
        };
        log_failure(result.await, "store a provider detection");
    }

    /// Drops the cached scopes of `provider` that are not in `keep` (a profile or project the CLI
    /// no longer reports).
    pub async fn prune(&self, provider: CloudProvider, keep: &[CloudScope]) {
        let snapshot = self.load(provider).await;
        for row in snapshot
            .scopes
            .iter()
            .filter(|row| !keep.contains(&row.scope))
        {
            let result = async {
                let key = state_key(&row.scope.key())?;
                self.state.table_delete(&scopes_table()?, &key).await
            };
            log_failure(result.await, "drop a scope listing");
        }
    }
}

fn log_failure<T>(result: OxiResult<T>, what: &str) {
    if let Err(error) = result {
        tracing::warn!(%error, "the cloud discovery cache could not {what}");
    }
}

fn scopes_table() -> OxiResult<StateTable> {
    StateTable::new(SCOPES_TABLE)
}

fn providers_table() -> OxiResult<StateTable> {
    StateTable::new(PROVIDERS_TABLE)
}

/// A state key for `raw`: the characters a key may hold stay, `:` and every other byte become
/// `:` and two hex digits. Injective, so two scopes never share a row (an AWS profile may hold
/// characters a key may not).
pub(super) fn state_key(raw: &str) -> OxiResult<StateKey> {
    let mut key = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/') {
            key.push(char::from(byte));
        } else {
            key.push_str(&format!(":{byte:02x}"));
        }
    }
    StateKey::new(key).map_err(|error| {
        OxiError::validation(format!("a cloud scope's cache key is not valid: {error}"))
    })
}
