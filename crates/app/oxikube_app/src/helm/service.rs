//! [`HelmService`]: a release revision's manifest, parsed once and mapped to resources.

use std::collections::VecDeque;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::helm::{ParsedManifest, ReleaseRef, parse_manifest};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::HelmPort;
use parking_lot::Mutex;

use super::kinds::KindScopes;
use super::lease::ReleaseResourcesLease;
use super::plan::ResourcePlan;
use crate::store::ResourceStore;

/// How many parsed revisions are kept.
const PARSED_KEPT: usize = 8;

/// Helm has no release uid; the revision's `updated` stamp tells a reinstalled release (whose
/// revisions restart at 1) from the one it replaced.
type Key = (ReleaseRef, u32, Timestamp);

struct Inner {
    helm: Arc<dyn HelmPort>,
    /// Oldest first.
    parsed: Mutex<VecDeque<(Key, Arc<ParsedManifest>)>>,
}

/// Maps a release's manifest to the resources it owns. Cheap to clone. See the
/// [module docs](super).
#[derive(Clone)]
pub struct HelmService {
    inner: Arc<Inner>,
}

impl HelmService {
    /// A service reading manifests through `helm` (the native reader or the CLI).
    pub fn new(helm: Arc<dyn HelmPort>) -> Self {
        Self {
            inner: Arc::new(Inner {
                helm,
                parsed: Mutex::new(VecDeque::new()),
            }),
        }
    }

    fn cached(&self, key: &Key) -> Option<Arc<ParsedManifest>> {
        let cache = self.inner.parsed.lock();
        cache.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    /// The parsed manifest of `release` at `revision`: read from the port and parsed off the
    /// calling thread the first time, then served from a small cache keyed by release,
    /// revision and `updated` stamp (a revision's manifest never changes while the release lives;
    /// after an uninstall and reinstall revisions restart, but the stamp differs). Hooks and per-document errors are in it.
    pub async fn manifest(
        &self,
        release: &ReleaseRef,
        revision: u32,
        updated: Timestamp,
    ) -> OxiResult<Arc<ParsedManifest>> {
        let key = (release.clone(), revision, updated);
        if let Some(hit) = self.cached(&key) {
            return Ok(hit);
        }
        let text = self.inner.helm.manifest(release, Some(revision)).await?;
        // A manifest can be megabytes: the YAML work belongs on a blocking thread.
        let parsed = tokio::task::spawn_blocking(move || parse_manifest(&text))
            .await
            .map_err(|e| OxiError::internal(format!("manifest parsing stopped: {e}")))?;
        let parsed = Arc::new(parsed);
        let mut cache = self.inner.parsed.lock();
        if !cache.iter().any(|(k, _)| *k == key) {
            cache.push_back((key, parsed.clone()));
            while cache.len() > PARSED_KEPT {
                cache.pop_front();
            }
        }
        Ok(parsed)
    }

    /// The objects of `release` at `revision` as `ResourceRef`s, mapped against what discovery
    /// serves (`kinds`). Parsing is cached; mapping is O(objects) and cheap.
    pub async fn plan(
        &self,
        release: &ReleaseRef,
        revision: u32,
        updated: Timestamp,
        kinds: &KindScopes,
    ) -> OxiResult<Arc<ResourcePlan>> {
        let parsed = self.manifest(release, revision, updated).await?;
        Ok(Arc::new(ResourcePlan::build(release, &parsed, kinds)))
    }

    /// Plans and subscribes in one step: the lease keeps the feeds open while the Resources tab
    /// shows them. Drop it when the tab closes.
    pub async fn open(
        &self,
        store: &ResourceStore,
        release: &ReleaseRef,
        revision: u32,
        updated: Timestamp,
        kinds: &KindScopes,
    ) -> OxiResult<ReleaseResourcesLease> {
        let plan = self.plan(release, revision, updated, kinds).await?;
        Ok(ReleaseResourcesLease::new(store, plan))
    }
}
