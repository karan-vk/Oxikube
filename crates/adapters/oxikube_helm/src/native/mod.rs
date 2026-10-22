//! The native release reader: [`NativeHelmReader`] implements the read half of
//! [`HelmPort`] by decoding Helm's own release records, so releases, history, values,
//! manifests and notes work without a `helm` binary (E16-S02).
//!
//! # How it reads
//!
//! * **List.** Per namespace, a metadata-only list of Secrets and ConfigMaps labelled
//!   `owner=helm` (both storage drivers, since a cluster may use either). Objects are grouped
//!   by `(namespace, release)` and the highest `version` label wins. Only the latest
//!   revision of each release is fetched and decoded, and only for its chart, app version,
//!   status and time; `FETCH_CONCURRENCY` at a time.
//! * **Detail.** `history`, `values`, `manifest` and `notes` list the one release's objects
//!   and decode what they need on demand.
//! * **Decode.** base64 (twice for Secrets), gzip (detected by its magic bytes), JSON;
//!   capped at 50 MiB inflated; run on tokio's blocking pool, never on an async worker or
//!   the UI thread. Results are cached by `(uid, resourceVersion)` (memory only), so a
//!   second list decodes only what changed.
//!
//! Callers run these methods through `oxikube_runtime::spawn_kube`. Decoded values and
//! manifests can hold credentials: they are returned, never logged or written anywhere;
//! errors name the object and the failed step, never payload text.
//!
//! # Not here
//!
//! Repos, charts, `template` and every mutation need the `helm` CLI adapter: this reader
//! returns [`HelmError::Unsupported`] for them and reports
//! [`HelmCapabilities::native_only`].

mod cache;
mod codec;
mod reader;
mod record;
mod storage;

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::OxiError;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::helm::{
    ChartHit, ChartRef, HelmCapabilities, HelmError, HelmOutput, HelmPort, InstallRequest, Release,
    ReleaseRef, RepoEntry, Revision, RollbackRequest, SearchQuery, ShowKind, UninstallRequest,
    UpgradeRequest, Values,
};
use oxikube_ports::resource::ResourceReader;

use self::reader::Store;
use self::record::{Record, release_row, revision_row};
use self::storage::{OWNER_SELECTOR, StoredRevision};

/// Reads the Helm releases of one cluster from their release Secrets and ConfigMaps.
///
/// Read-only: it needs only `list`/`get` on Secrets and ConfigMaps, and holds a
/// [`ResourceReader`], never a writer.
pub struct NativeHelmReader {
    cluster: ClusterId,
    store: Store,
}

impl NativeHelmReader {
    /// A reader for `cluster`, reading through that cluster's resource port.
    pub fn new(cluster: ClusterId, reader: Arc<dyn ResourceReader>) -> Self {
        Self {
            cluster,
            store: Store::new(reader),
        }
    }

    fn check_cluster(&self, cluster: &ClusterId) -> OxiResult<()> {
        if *cluster == self.cluster {
            Ok(())
        } else {
            Err(OxiError::validation(format!(
                "this Helm reader serves cluster {}, not {cluster}",
                self.cluster
            )))
        }
    }

    /// Every stored revision of `release`, newest first (one per revision number).
    async fn revisions(&self, release: &ReleaseRef) -> OxiResult<Vec<StoredRevision>> {
        self.check_cluster(&release.cluster)?;
        if !storage::is_release_name(&release.name) {
            return Err(not_found(release, None));
        }
        let selector = format!("{OWNER_SELECTOR},name={}", release.name);
        let stored = self.store.list(Some(&release.namespace), &selector).await?;
        let stored: Vec<_> = stored
            .into_iter()
            .filter(|s| s.release == release.name && *s.namespace == *release.namespace)
            .collect();
        if stored.is_empty() {
            return Err(not_found(release, None));
        }
        Ok(storage::history_order(stored))
    }

    /// The decoded record of `revision` of `release` (latest when `None`).
    async fn record(&self, release: &ReleaseRef, revision: Option<u32>) -> OxiResult<Arc<Record>> {
        let revisions = self.revisions(release).await?;
        let stored = match revision {
            None => revisions.first(),
            Some(n) => revisions.iter().find(|s| s.revision == n),
        }
        .ok_or_else(|| not_found(release, revision))?;
        self.store.record(stored).await
    }
}

fn not_found(release: &ReleaseRef, revision: Option<u32>) -> OxiError {
    let what = match revision {
        Some(n) => format!("{release} revision {n}"),
        None => release.to_string(),
    };
    HelmError::ReleaseNotFound(what).into()
}

fn unsupported<T>() -> OxiResult<T> {
    Err(HelmError::Unsupported.into())
}

#[async_trait]
impl HelmPort for NativeHelmReader {
    async fn list_releases(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<Vec<Release>> {
        self.check_cluster(cluster)?;
        let scopes: Vec<Option<&str>> = match namespaces {
            NamespaceSelection::All => vec![None],
            NamespaceSelection::Set(set) => set.iter().map(|ns| Some(ns.as_str())).collect(),
        };
        let stored = reader::list_many(&self.store, &scopes, OWNER_SELECTOR).await?;
        let latest = storage::latest_per_release(stored);
        let headers = self.store.headers(&latest).await;
        Ok(latest
            .iter()
            .zip(headers)
            .map(|(stored, header)| {
                let reference =
                    ReleaseRef::new(cluster.clone(), &*stored.namespace, stored.release.as_str());
                release_row(reference, stored, header.as_deref())
            })
            .collect())
    }

    async fn history(&self, release: &ReleaseRef) -> OxiResult<Vec<Revision>> {
        let revisions = self.revisions(release).await?;
        let headers = self.store.headers(&revisions).await;
        Ok(revisions
            .iter()
            .zip(headers)
            .map(|(stored, header)| revision_row(stored, header.as_deref()))
            .collect())
    }

    async fn values(
        &self,
        release: &ReleaseRef,
        revision: Option<u32>,
        all: bool,
    ) -> OxiResult<Values> {
        let record = self.record(release, revision).await?;
        Ok(if all {
            record.computed_values()
        } else {
            record.user_values.clone()
        })
    }

    async fn manifest(&self, release: &ReleaseRef, revision: Option<u32>) -> OxiResult<String> {
        Ok(self.record(release, revision).await?.manifest.to_string())
    }

    async fn notes(
        &self,
        release: &ReleaseRef,
        revision: Option<u32>,
    ) -> OxiResult<Option<String>> {
        let record = self.record(release, revision).await?;
        Ok(record.notes.as_deref().map(str::to_owned))
    }

    async fn repos(&self) -> OxiResult<Vec<RepoEntry>> {
        unsupported()
    }

    async fn search(&self, _query: &SearchQuery) -> OxiResult<Vec<ChartHit>> {
        unsupported()
    }

    async fn show(&self, _chart: &ChartRef, _kind: ShowKind) -> OxiResult<String> {
        unsupported()
    }

    async fn template(&self, _request: &InstallRequest) -> OxiResult<String> {
        unsupported()
    }

    async fn install(&self, _request: &InstallRequest) -> OxiResult<HelmOutput> {
        unsupported()
    }

    async fn upgrade(&self, _request: &UpgradeRequest) -> OxiResult<HelmOutput> {
        unsupported()
    }

    async fn rollback(&self, _request: &RollbackRequest) -> OxiResult<()> {
        unsupported()
    }

    async fn uninstall(&self, _request: &UninstallRequest) -> OxiResult<()> {
        unsupported()
    }

    fn capabilities(&self) -> HelmCapabilities {
        HelmCapabilities::native_only()
    }
}
