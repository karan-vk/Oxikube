//! [`OpenApiSchemas`]: [`SchemaPort`] over the cluster's OpenAPI v3 endpoint.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use kube::Client;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::schema::{JsonSchema, root_schema_for};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{FsPort, SchemaPort};
use parking_lot::Mutex;
use tracing::{debug, warn};

use super::cache::{DiskCache, Key};
use super::fetch::{fetch_document, fetch_index, fetch_server_version};
use super::index::{Index, IndexEntry};

/// Default per-request deadline for the index and group-document fetches.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

/// How old the in-memory index must be before a miss re-reads it (a CRD that
/// was just created reaches the server's OpenAPI document a moment after its
/// discovery event).
pub const DEFAULT_REFRESH_ON_MISS_SECS: u64 = 5;

/// Settings of [`OpenApiSchemas`].
#[derive(Debug, Clone)]
pub struct OpenApiConfig {
    /// Deadline for one index or group-document fetch.
    pub request_timeout: Duration,
    /// A lookup that finds no schema re-reads the index first when the one in
    /// memory is at least this old, so a kind added since is found without an
    /// explicit invalidate. Younger indexes answer `NotFound` immediately.
    pub refresh_on_miss_after: Duration,
}

impl Default for OpenApiConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
            refresh_on_miss_after: Duration::from_secs(DEFAULT_REFRESH_ON_MISS_SECS),
        }
    }
}

/// The index and server version read together, and when.
struct Loaded {
    index: Index,
    /// The server's `gitVersion` (`unknown` when `/version` could not be read).
    server_version: String,
    at: Instant,
}

/// What `invalidate` clears. `epoch` makes a fetch that was in flight during
/// an invalidate drop its result instead of caching stale data.
#[derive(Default)]
struct Memory {
    loaded: Option<Arc<Loaded>>,
    schemas: HashMap<Gvk, Arc<JsonSchema>>,
    epoch: u64,
}

/// `SchemaPort` over OpenAPI v3: one instance serves one cluster session.
///
/// Built with the session's [`Client`](kube::Client) and its cluster id.
/// Fetching, parsing and flattening run on the caller's task (never the UI
/// thread: the editor calls through `spawn_kube`), with the CPU-bound parse on
/// the blocking pool; concurrent callers of one group-version share one fetch.
pub struct OpenApiSchemas {
    client: Client,
    cluster: ClusterId,
    disk: Option<DiskCache>,
    config: OpenApiConfig,
    memory: Mutex<Memory>,
    /// Serialises the index load so two first callers fetch it once.
    index_gate: tokio::sync::Mutex<()>,
    /// One gate per group-version document (single-flight per document).
    groups: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl std::fmt::Debug for OpenApiSchemas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenApiSchemas")
            .field("cluster", &self.cluster)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl OpenApiSchemas {
    /// Serves `client`'s cluster, caching in memory only.
    pub fn new(client: Client, cluster: ClusterId) -> Self {
        Self::with_config(client, cluster, OpenApiConfig::default())
    }

    /// As [`new`](Self::new) with explicit settings.
    pub fn with_config(client: Client, cluster: ClusterId, config: OpenApiConfig) -> Self {
        Self {
            client,
            cluster,
            disk: None,
            config,
            memory: Mutex::default(),
            index_gate: tokio::sync::Mutex::new(()),
            groups: Mutex::default(),
        }
    }

    /// Also caches raw group documents under `cache_dir` through `fs`, keyed by
    /// cluster, server version and index hash (see the module docs).
    #[must_use]
    pub fn with_disk_cache(mut self, fs: Arc<dyn FsPort>, cache_dir: PathBuf) -> Self {
        self.disk = Some(DiskCache::new(fs, cache_dir));
        self
    }

    fn cached(&self, gvk: &Gvk) -> Option<Arc<JsonSchema>> {
        self.memory.lock().schemas.get(gvk).cloned()
    }

    /// The index and server version, fetched once and shared.
    async fn loaded(&self) -> OxiResult<Arc<Loaded>> {
        if let Some(loaded) = self.memory.lock().loaded.clone() {
            return Ok(loaded);
        }
        let _gate = self.index_gate.lock().await;
        if let Some(loaded) = self.memory.lock().loaded.clone() {
            return Ok(loaded);
        }
        let epoch = self.memory.lock().epoch;
        let timeout = self.config.request_timeout;
        let (index, version) = tokio::join!(
            fetch_index(&self.client, timeout),
            fetch_server_version(&self.client, timeout)
        );
        let loaded = Arc::new(Loaded {
            index: index?,
            server_version: version.unwrap_or_else(|| "unknown".to_owned()),
            at: Instant::now(),
        });
        let mut memory = self.memory.lock();
        if memory.epoch == epoch {
            memory.loaded = Some(loaded.clone());
        }
        Ok(loaded)
    }

    /// Looks `gvk` up through the cached index. `NotFound` when the server
    /// lists no document or the document names no such kind.
    async fn resolve(&self, gvk: &Gvk) -> OxiResult<Arc<JsonSchema>> {
        let loaded = self.loaded().await?;
        let Some(entry) = loaded.index.entry_for(&gvk.group, &gvk.version) else {
            return Err(OxiError::not_found(format!(
                "no OpenAPI v3 document lists {gvk}"
            )));
        };
        let gate = self
            .groups
            .lock()
            .entry(Index::key_for(&gvk.group, &gvk.version))
            .or_default()
            .clone();
        let _flight = gate.lock().await;
        if let Some(hit) = self.cached(gvk) {
            return Ok(hit);
        }
        let epoch = self.memory.lock().epoch;
        let schema = Arc::new(self.load_group(&loaded, entry, gvk).await?);
        let mut memory = self.memory.lock();
        if memory.epoch == epoch {
            memory.schemas.insert(gvk.clone(), schema.clone());
        }
        Ok(schema)
    }

    /// The flattened root of `gvk` from the disk cache or, failing that, the
    /// server (then stored on disk).
    async fn load_group(
        &self,
        loaded: &Loaded,
        entry: &IndexEntry,
        gvk: &Gvk,
    ) -> OxiResult<JsonSchema> {
        let key = Key {
            server_version: &loaded.server_version,
            group_version: &Index::key_for(&gvk.group, &gvk.version),
            hash: &entry.hash,
        };
        if let Some(disk) = &self.disk {
            if let Some(bytes) = disk.read(&self.cluster, &key).await {
                match flatten_root(bytes, gvk).await {
                    Ok((root, _)) => {
                        debug!(kind = %gvk, "openapi: group document from disk cache");
                        return root.ok_or_else(|| no_schema(gvk));
                    }
                    Err(_) => warn!(kind = %gvk, "openapi: unreadable disk cache entry ignored"),
                }
            }
        }
        let text = fetch_document(&self.client, &entry.url, self.config.request_timeout).await?;
        let (root, bytes) = flatten_root(text.into_bytes(), gvk)
            .await
            .map_err(|e| OxiError::internal(format!("openapi: group document is not JSON: {e}")))?;
        if let Some(disk) = &self.disk {
            if let Err(err) = disk.write(&self.cluster, &key, &bytes).await {
                warn!(error = %err, kind = %gvk, "openapi: disk cache write failed");
            }
        }
        root.ok_or_else(|| no_schema(gvk))
    }

    /// Re-reads the index on the next lookup when the one in memory is old
    /// enough (see [`OpenApiConfig::refresh_on_miss_after`]). Whether it did.
    fn forget_old_index(&self) -> bool {
        let mut memory = self.memory.lock();
        let old = memory
            .loaded
            .as_ref()
            .is_some_and(|l| l.at.elapsed() >= self.config.refresh_on_miss_after);
        if old {
            memory.loaded = None;
        }
        old
    }

    fn check_cluster(&self, cluster: &ClusterId) -> OxiResult<()> {
        if *cluster == self.cluster {
            Ok(())
        } else {
            Err(OxiError::validation(format!(
                "this schema source serves cluster {}, not {cluster}",
                self.cluster
            )))
        }
    }
}

fn no_schema(gvk: &Gvk) -> OxiError {
    OxiError::not_found(format!("no OpenAPI v3 schema names {gvk}"))
}

/// Parses a group document and flattens `gvk`'s root on the blocking pool (a
/// large document takes tens of milliseconds, too long for an async worker).
/// Hands the bytes back so the caller can store them without a copy; the error
/// is why the document did not parse.
async fn flatten_root(bytes: Vec<u8>, gvk: &Gvk) -> Result<(Option<JsonSchema>, Vec<u8>), String> {
    let gvk = gvk.clone();
    tokio::task::spawn_blocking(move || {
        let document: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        Ok((root_schema_for(&document, &gvk), bytes))
    })
    .await
    .map_err(|e| format!("parse task ended: {e}"))?
}

#[async_trait]
impl SchemaPort for OpenApiSchemas {
    async fn schema_for(&self, cluster: &ClusterId, gvk: &Gvk) -> OxiResult<Arc<JsonSchema>> {
        self.check_cluster(cluster)?;
        if let Some(hit) = self.cached(gvk) {
            return Ok(hit);
        }
        match self.resolve(gvk).await {
            Err(err) if err.kind() == ErrorKind::NotFound && self.forget_old_index() => {
                self.resolve(gvk).await
            }
            other => other,
        }
    }

    async fn invalidate(&self, cluster: &ClusterId) -> OxiResult<()> {
        self.check_cluster(cluster)?;
        let mut memory = self.memory.lock();
        memory.epoch += 1;
        memory.loaded = None;
        memory.schemas.clear();
        Ok(())
    }
}
