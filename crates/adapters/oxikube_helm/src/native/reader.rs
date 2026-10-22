//! Reading release objects through the resource port: metadata-only listing of both
//! drivers, and on-demand fetch + decode (off the async workers) with the cache in front.

use std::sync::Arc;

use futures::future::try_join_all;
use oxikube_domain::helm::HelmError;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::resource::{ListOptions, ResourceReader};

use super::cache::DecodeCache;
use super::codec;
use super::record::{Header, RawHeader, RawRecord, Record};
use super::storage::{self, Driver, SECRET_TYPE, StoredRevision};

/// Page size of the metadata lists.
const PAGE_SIZE: u32 = 500;

/// Release objects fetched and decoded at once.
pub(crate) const FETCH_CONCURRENCY: usize = 16;

/// The release store of one cluster.
pub(crate) struct Store {
    reader: Arc<dyn ResourceReader>,
    cache: DecodeCache,
}

impl Store {
    pub fn new(reader: Arc<dyn ResourceReader>) -> Self {
        Self {
            reader,
            cache: DecodeCache::default(),
        }
    }

    /// The stored revisions matching `selector` in `namespace` (`None`: all namespaces),
    /// from both drivers, metadata only. A driver the caller may not list (RBAC) is skipped
    /// as long as the other one can be listed.
    pub async fn list(
        &self,
        namespace: Option<&str>,
        selector: &str,
    ) -> OxiResult<Vec<StoredRevision>> {
        let (secrets, configmaps) = futures::join!(
            self.list_driver(Driver::Secret, namespace, selector),
            self.list_driver(Driver::ConfigMap, namespace, selector),
        );
        match (secrets, configmaps) {
            (Ok(mut a), Ok(b)) => {
                a.extend(b);
                Ok(a)
            }
            (Ok(only), Err(e)) | (Err(e), Ok(only)) if e.kind() == ErrorKind::Forbidden => {
                tracing::debug!(error = %e, "helm: one release storage driver is not readable");
                Ok(only)
            }
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    }

    async fn list_driver(
        &self,
        driver: Driver,
        namespace: Option<&str>,
        selector: &str,
    ) -> OxiResult<Vec<StoredRevision>> {
        let kind = driver.gvk();
        let mut options = ListOptions::default().labels(selector).limit(PAGE_SIZE);
        let mut out = Vec::new();
        loop {
            let page = self
                .reader
                .list_metadata(&kind, namespace, &options)
                .await?;
            out.extend(
                page.items
                    .iter()
                    .filter_map(|meta| storage::parse_stored(meta, driver)),
            );
            match page.continue_token.filter(|t| !t.is_empty()) {
                Some(token) => options = options.continue_from(token),
                None => return Ok(out),
            }
        }
    }

    /// The header of `stored`, decoded once per `(uid, resourceVersion)`.
    pub async fn header(&self, stored: &StoredRevision) -> OxiResult<Arc<Header>> {
        if let Some(header) = stored.cache_key.as_ref().and_then(|k| self.cache.header(k)) {
            return Ok(header);
        }
        let data = self.fetch_data(stored).await?;
        let driver = stored.driver;
        let header = offload(move || codec::decode::<RawHeader>(driver, &data)).await??;
        let header = Arc::new(header.into_header());
        if let Some(key) = stored.cache_key.clone() {
            self.cache.put_header(key, header.clone());
        }
        Ok(header)
    }

    /// The full record of `stored`, decoded on demand and cached.
    pub async fn record(&self, stored: &StoredRevision) -> OxiResult<Arc<Record>> {
        if let Some(record) = stored.cache_key.as_ref().and_then(|k| self.cache.record(k)) {
            return Ok(record);
        }
        let data = self.fetch_data(stored).await?;
        let driver = stored.driver;
        let record = offload(move || codec::decode::<RawRecord>(driver, &data)).await??;
        let record = Arc::new(record.into_record());
        if let Some(key) = stored.cache_key.clone() {
            self.cache.put_record(key, record.clone());
        }
        Ok(record)
    }

    /// Headers of many revisions, `FETCH_CONCURRENCY` at a time, in input order. A revision
    /// whose header cannot be read gives `None` (logged without payload) so one bad record
    /// never hides the others.
    pub async fn headers(&self, stored: &[StoredRevision]) -> Vec<Option<Arc<Header>>> {
        use futures::StreamExt as _;
        // Futures collected first: a closure inside the stream trips the `Send` check of
        // the `async_trait` callers (higher-ranked lifetime inference).
        let pending: Vec<_> = stored.iter().map(|s| self.header_or_none(s)).collect();
        futures::stream::iter(pending)
            .buffered(FETCH_CONCURRENCY)
            .collect()
            .await
    }

    async fn header_or_none(&self, stored: &StoredRevision) -> Option<Arc<Header>> {
        match self.header(stored).await {
            Ok(header) => Some(header),
            Err(e) => {
                tracing::warn!(
                    namespace = %stored.namespace, release = %stored.release,
                    revision = stored.revision, error = %e,
                    "helm: release record unreadable; showing labels only"
                );
                None
            }
        }
    }

    /// Reads `data.release` of the object behind `stored`.
    async fn fetch_data(&self, stored: &StoredRevision) -> OxiResult<String> {
        let object = self
            .reader
            .get(
                &stored.driver.gvk(),
                Some(&stored.namespace),
                &stored.object,
            )
            .await
            .map_err(|e| match e.kind() {
                ErrorKind::NotFound => OxiError::from(HelmError::ReleaseNotFound(format!(
                    "{}/{} revision {}",
                    stored.namespace, stored.release, stored.revision
                ))),
                _ => e,
            })?;
        if stored.driver == Driver::Secret
            && object.get_str("/type").is_some_and(|t| t != SECRET_TYPE)
        {
            return Err(HelmError::Parse(format!(
                "Secret {} is not of type {SECRET_TYPE}",
                stored.object
            ))
            .into());
        }
        object
            .get_str("/data/release")
            .map(str::to_owned)
            .ok_or_else(|| {
                HelmError::Parse(format!("{} has no release data", stored.object)).into()
            })
    }
}

/// Runs CPU-bound decoding on tokio's blocking pool when there is a runtime (always, under
/// `oxikube_runtime::spawn_kube`), inline otherwise.
async fn offload<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> OxiResult<T> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle
            .spawn_blocking(work)
            .await
            .map_err(|e| OxiError::internal(format!("helm: decoding was interrupted ({e})"))),
        Err(_) => Ok(work()),
    }
}

/// Concurrent [`Store::list`] over several namespaces.
pub(crate) async fn list_many(
    store: &Store,
    namespaces: &[Option<&str>],
    selector: &str,
) -> OxiResult<Vec<StoredRevision>> {
    let lists = try_join_all(namespaces.iter().map(|ns| store.list(*ns, selector))).await?;
    Ok(lists.into_iter().flatten().collect())
}
