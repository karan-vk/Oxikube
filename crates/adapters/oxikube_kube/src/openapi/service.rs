//! [`OpenApiSchemas`]: [`SchemaPort`] over the cluster's OpenAPI v3 endpoint.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kube::Client;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::schema::{JsonSchema, root_schema_for};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{FsPort, SchemaPort};
use tracing::{debug, warn};

use super::cache;
use super::index::{Index, parse_index};
use crate::auth::classify;

/// Default per-request deadline for the index and group-document fetches.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

/// Settings of [`OpenApiSchemas`].
#[derive(Debug, Clone)]
pub struct OpenApiConfig {
    /// Deadline for one index or group-document fetch.
    pub request_timeout: Duration,
}

impl Default for OpenApiConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
        }
    }
}

/// In-memory state behind the fetch mutex.
#[derive(Default)]
struct State {
    /// The `/openapi/v3` index, loaded on first use.
    index: Option<Index>,
    /// Flattened schemas by (cluster, GVK).
    schemas: HashMap<(ClusterId, Gvk), Arc<JsonSchema>>,
}

/// `SchemaPort` over OpenAPI v3: one instance serves one cluster session.
///
/// Built with the session's [`Client`](kube::Client) and a filesystem for the
/// disk cache. All fetching runs on the caller's task (never the UI thread:
/// the editor calls through `spawn_kube`); concurrent callers share one fetch
/// through the state mutex.
pub struct OpenApiSchemas {
    client: Client,
    fs: Arc<dyn FsPort>,
    cache_dir: PathBuf,
    config: OpenApiConfig,
    state: tokio::sync::Mutex<State>,
}

impl std::fmt::Debug for OpenApiSchemas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenApiSchemas")
            .field("cache_dir", &self.cache_dir)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl OpenApiSchemas {
    /// Serves `client`'s cluster, caching raw documents under `cache_dir`.
    pub fn new(client: Client, fs: Arc<dyn FsPort>, cache_dir: PathBuf) -> Self {
        Self::with_config(client, fs, cache_dir, OpenApiConfig::default())
    }

    /// As [`new`](Self::new) with explicit settings.
    pub fn with_config(
        client: Client,
        fs: Arc<dyn FsPort>,
        cache_dir: PathBuf,
        config: OpenApiConfig,
    ) -> Self {
        Self {
            client,
            fs,
            cache_dir,
            config,
            state: tokio::sync::Mutex::new(State::default()),
        }
    }

    /// The group-version key of `gvk` as the index lists it (`api/v1`,
    /// `apis/apps/v1`).
    fn group_version_of(gvk: &Gvk) -> String {
        Index::key_for(&gvk.group, &gvk.version)
    }
}

#[async_trait]
impl SchemaPort for OpenApiSchemas {
    async fn schema_for(&self, cluster: &ClusterId, gvk: &Gvk) -> OxiResult<Arc<JsonSchema>> {
        let mut state = self.state.lock().await;
        if let Some(hit) = state.schemas.get(&(cluster.clone(), gvk.clone())) {
            return Ok(hit.clone());
        }
        if state.index.is_none() {
            state.index = Some(fetch_index(&self.client, self.config.request_timeout).await?);
        }
        let group_version = Self::group_version_of(gvk);
        let entry = state
            .index
            .as_ref()
            .and_then(|index| index.entry_for(&gvk.group, &gvk.version))
            .cloned();
        let Some(entry) = entry else {
            return Err(OxiError::not_found(format!(
                "no OpenAPI v3 document lists {gvk}"
            )));
        };
        let document = match cache::read(
            self.fs.as_ref(),
            &self.cache_dir,
            cluster,
            &group_version,
            &entry.hash,
        )
        .await?
        {
            Some(hit) => {
                debug!(kind = %gvk, "openapi: group document from disk cache");
                hit.document
            }
            None => {
                let document =
                    fetch_document(&self.client, &entry.url, self.config.request_timeout).await?;
                if let Err(err) = cache::write(
                    self.fs.as_ref(),
                    &self.cache_dir,
                    cluster,
                    &group_version,
                    &entry.hash,
                    &document,
                )
                .await
                {
                    warn!(error = %err, kind = %gvk, "openapi: disk cache write failed");
                }
                document
            }
        };
        let Some(schema) = root_schema_for(&document, gvk) else {
            return Err(OxiError::not_found(format!(
                "no OpenAPI v3 schema names {gvk}"
            )));
        };
        let shared = Arc::new(schema);
        state
            .schemas
            .insert((cluster.clone(), gvk.clone()), shared.clone());
        Ok(shared)
    }

    async fn invalidate(&self, cluster: &ClusterId) -> OxiResult<()> {
        {
            let mut state = self.state.lock().await;
            state.schemas.retain(|(owner, _), _| owner != cluster);
            state.index = None;
        }
        cache::remove_cluster(self.fs.as_ref(), &self.cache_dir, cluster).await
    }
}

/// `GET /openapi/v3`: the index of group-version documents. A 404 means the
/// server predates OpenAPI v3 (aggregated discovery needs 1.27+ for the full
/// set): [`Unsupported`](ErrorKind::Unsupported), never swallowed.
async fn fetch_index(client: &Client, timeout: Duration) -> OxiResult<Index> {
    let text = get(client, "/openapi/v3", timeout).await.map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            OxiError::unsupported("the API server has no /openapi/v3 endpoint")
        } else {
            err
        }
    })?;
    let document: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| OxiError::internal(format!("openapi: index is not JSON: {e}")))?;
    Ok(parse_index(&document))
}

/// `GET` one group-version document. Bodies are public API schemas, but only
/// their size is ever logged.
async fn fetch_document(
    client: &Client,
    url: &str,
    timeout: Duration,
) -> OxiResult<serde_json::Value> {
    let text = get(client, url, timeout).await?;
    let bytes = text.len();
    let document: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| OxiError::internal(format!("openapi: group document is not JSON: {e}")))?;
    debug!(url = %path_for_logs(url), bytes, "openapi: group document fetched");
    Ok(document)
}

/// `GET` `url` as text within `timeout`. The URL is server-built (index) or a
/// fixed path; error messages carry the path but never a body.
async fn get(client: &Client, url: &str, timeout: Duration) -> OxiResult<String> {
    let request = http::Request::get(url)
        .body(Vec::new())
        .map_err(|e| OxiError::validation(format!("openapi: bad request path: {e}")))?;
    let text = tokio::time::timeout(timeout, client.request_text(request))
        .await
        .map_err(|_| OxiError::timeout(format!("openapi: GET {url} did not complete in time")))?
        .map_err(|e| classify_status(&e))?;
    Ok(text)
}

/// Maps a fetch failure: 404 is `NotFound` (the caller refines the index case
/// to `Unsupported`); everything else follows the adapter's table.
fn classify_status(err: &kube::Error) -> OxiError {
    match err {
        kube::Error::Api(status) if status.code == 404 => {
            OxiError::not_found("the API server has no such OpenAPI v3 document")
        }
        _ => classify(err),
    }
}

/// The URL path without its query, for short stable logs (the `?hash=` query
/// adds nothing to read).
fn path_for_logs(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}
