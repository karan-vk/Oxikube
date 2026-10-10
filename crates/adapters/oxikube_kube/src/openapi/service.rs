//! [`OpenApiSchemas`]: [`SchemaPort`] over the cluster's OpenAPI v3 endpoint.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use kube::Client;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::schema::{JsonSchema, root_schema_for};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{FsPort, SchemaPort};
use parking_lot::Mutex;
use tracing::{debug, warn};

use super::cache::{DiskCache, Key, UNKNOWN_VERSION};
use super::config::{OpenApiConfig, VERSION_TIMEOUT};
use super::fetch::{fetch_document, fetch_index, fetch_server_version, no_openapi_v3};
use super::index::{Index, IndexEntry};

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
    /// Raw group documents (by index key, with the hash they were read under) that could not
    /// be kept on disk: no disk cache, an unwritable one, or a server without hashes. Without
    /// them every further kind of the group would download the document again.
    documents: HashMap<String, (String, Arc<Vec<u8>>)>,
    /// When the server last answered "no `/openapi/v3`": remembered for
    /// [`OpenApiConfig::refresh_on_miss_after`], so a validator asking on every edit does not
    /// repeat doomed requests, while a gateway that blipped a 404 recovers on its own.
    unsupported_at: Option<Instant>,
    /// Kinds the server had no schema for, and when: answered `NotFound` without a request for
    /// [`OpenApiConfig::refresh_on_miss_after`] (a CRD kind with no schema is asked about on
    /// every edit).
    misses: HashMap<Gvk, Instant>,
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
    /// One gate per group-version document (single-flight per document). (`pub(super)`: tests
    /// hold one to park a lookup between the index and the document.)
    pub(super) groups: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
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
        if let Some(known) = self.known_index()? {
            return Ok(known);
        }
        let _gate = self.index_gate.lock().await;
        if let Some(known) = self.known_index()? {
            return Ok(known);
        }
        let epoch = self.memory.lock().epoch;
        let timeout = self.config.request_timeout;
        // Concurrent, but an index failure returns at once instead of waiting for `/version`.
        let (index, version) = futures::try_join!(fetch_index(&self.client, timeout), async {
            // The version only keys the disk cache, so without one it is not asked, and it gets
            // a short deadline: a slow `/version` must not stall the first schema.
            Ok(match self.disk {
                Some(_) => fetch_server_version(&self.client, timeout.min(VERSION_TIMEOUT)).await,
                None => None,
            })
        })
        .inspect_err(|err| {
            if err.kind() == ErrorKind::Unsupported {
                let mut memory = self.memory.lock();
                if memory.epoch == epoch {
                    memory.unsupported_at = Some(Instant::now());
                }
            }
        })?;
        let loaded = Arc::new(Loaded {
            index,
            server_version: version.unwrap_or_else(|| UNKNOWN_VERSION.to_owned()),
            at: Instant::now(),
        });
        let mut memory = self.memory.lock();
        if memory.epoch == epoch {
            memory.loaded = Some(loaded.clone());
        }
        Ok(loaded)
    }

    /// The index in memory, `Unsupported` when the server is known to lack OpenAPI v3, or
    /// `None` when it has not been read yet.
    fn known_index(&self) -> OxiResult<Option<Arc<Loaded>>> {
        let memory = self.memory.lock();
        let recent = memory
            .unsupported_at
            .is_some_and(|at| at.elapsed() < self.config.refresh_on_miss_after);
        if recent {
            return Err(no_openapi_v3());
        }
        Ok(memory.loaded.clone())
    }

    /// Looks `gvk` up through the cached index. `NotFound` when the server
    /// lists no document or the document names no such kind.
    async fn resolve(&self, gvk: &Gvk) -> OxiResult<Arc<JsonSchema>> {
        // Before any await: an `invalidate` from here on discards this lookup's result.
        let epoch = self.memory.lock().epoch;
        let loaded = self.loaded().await?;
        let Some(entry) = loaded.index.entry_for(&gvk.group, &gvk.version) else {
            return Err(OxiError::not_found(format!(
                "no OpenAPI v3 document lists {gvk}"
            )));
        };
        let group_version = Index::key_for(&gvk.group, &gvk.version);
        let gate = self
            .groups
            .lock()
            .entry(group_version.clone())
            .or_default()
            .clone();
        let _flight = gate.lock().await;
        if let Some(hit) = self.cached(gvk) {
            return Ok(hit);
        }
        let schema = Arc::new(
            self.load_group(&loaded, entry, group_version, gvk, epoch)
                .await?,
        );
        let mut memory = self.memory.lock();
        if memory.epoch == epoch {
            memory.schemas.insert(gvk.clone(), schema.clone());
        }
        Ok(schema)
    }

    /// The flattened root of `gvk` from the document held in memory or on disk or, failing
    /// those, from the server (then kept on disk, or in memory when that is not possible).
    async fn load_group(
        &self,
        loaded: &Loaded,
        entry: &IndexEntry,
        group_version: String,
        gvk: &Gvk,
        epoch: u64,
    ) -> OxiResult<JsonSchema> {
        let key = Key {
            server_version: &loaded.server_version,
            group_version: &group_version,
            hash: &entry.hash,
        };
        let held = self
            .memory
            .lock()
            .documents
            .get(&group_version)
            .filter(|(hash, _)| *hash == entry.hash)
            .map(|(_, bytes)| bytes.clone());
        if let Some(bytes) = held {
            if let Ok(root) = flatten_root(bytes, gvk).await {
                return root.ok_or_else(|| no_schema(gvk));
            }
        }
        if let Some(disk) = &self.disk {
            if let Some(bytes) = disk.read(&self.cluster, &key).await {
                match flatten_root(Arc::new(bytes), gvk).await {
                    Ok(root) => {
                        debug!(kind = %gvk, "openapi: group document from disk cache");
                        return root.ok_or_else(|| no_schema(gvk));
                    }
                    Err(_) => warn!(kind = %gvk, "openapi: unreadable disk cache entry ignored"),
                }
            }
        }
        let text = fetch_document(&self.client, &entry.url, self.config.request_timeout).await?;
        let bytes = Arc::new(text.into_bytes());
        let root = flatten_root(bytes.clone(), gvk)
            .await
            .map_err(|e| OxiError::internal(format!("openapi: group document is not JSON: {e}")))?;
        let stored = match &self.disk {
            Some(disk) if !entry.hash.is_empty() => {
                match disk.write(&self.cluster, &key, &bytes).await {
                    Ok(()) => true,
                    Err(err) => {
                        warn!(error = %err, kind = %gvk, "openapi: disk cache write failed");
                        false
                    }
                }
            }
            _ => false,
        };
        if !stored {
            let mut memory = self.memory.lock();
            if memory.epoch == epoch {
                memory
                    .documents
                    .insert(group_version, (entry.hash.clone(), bytes));
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

    /// Whether the server was found to have no schema for `gvk` within the miss window.
    fn recent_miss(&self, gvk: &Gvk) -> bool {
        self.memory
            .lock()
            .misses
            .get(gvk)
            .is_some_and(|at| at.elapsed() < self.config.refresh_on_miss_after)
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
/// The error is why the document did not parse.
async fn flatten_root(bytes: Arc<Vec<u8>>, gvk: &Gvk) -> Result<Option<JsonSchema>, String> {
    let gvk = gvk.clone();
    tokio::task::spawn_blocking(move || {
        let document: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        Ok(root_schema_for(&document, &gvk))
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
        if self.recent_miss(gvk) {
            return Err(no_schema(gvk));
        }
        // Before any await, like the other writes: a miss that an `invalidate` overtook was
        // answered from the old index and must not hide the re-read the invalidate asked for.
        let epoch = self.memory.lock().epoch;
        let result = match self.resolve(gvk).await {
            Err(err) if err.kind() == ErrorKind::NotFound && self.forget_old_index() => {
                self.resolve(gvk).await
            }
            other => other,
        };
        if matches!(&result, Err(err) if err.kind() == ErrorKind::NotFound) {
            let mut memory = self.memory.lock();
            if memory.epoch == epoch {
                memory.misses.insert(gvk.clone(), Instant::now());
            }
        }
        result
    }

    async fn invalidate(&self, cluster: &ClusterId) -> OxiResult<()> {
        self.check_cluster(cluster)?;
        let mut memory = self.memory.lock();
        memory.epoch += 1;
        memory.loaded = None;
        memory.schemas.clear();
        memory.documents.clear();
        memory.unsupported_at = None;
        memory.misses.clear();
        Ok(())
    }
}
