//! [`ReleasesService`]: reads of one cluster's releases for the Releases view and the tools.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::helm::{Release, ReleaseRef, Revision, Values, mask_notes, mask_values};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::HelmPort;
use parking_lot::Mutex;
use tokio::sync::watch;

use super::source::ReleaseSource;
use super::watch::ReleaseWatch;
use crate::helm::{HelmService, ReleaseResourcesLease};

/// How much of a release the caller may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclosure {
    /// Always masked: tools and agents, whatever the user revealed in the view.
    Masked,
    /// Clear when the user revealed the release (`helm::RevealValues`), else masked: the view.
    Granted,
}

/// A release's values as text, for the Values tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValuesView {
    /// The YAML text. Credential-looking values are `********` unless [`revealed`](Self::revealed).
    pub text: Arc<str>,
    /// How many values are (or, once revealed, would be) masked.
    pub sensitive: usize,
    /// Whether the text shows them in clear.
    pub revealed: bool,
    /// The release has no values of this kind (no user-supplied values).
    pub empty: bool,
}

/// A release's manifest as text, for the Manifest tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestView {
    /// The multi-document YAML. Secret documents are masked unless the release is revealed.
    pub text: Arc<str>,
    /// Whether Secret data is masked in `text`.
    pub masked: bool,
}

struct Inner {
    source: Arc<dyn ReleaseSource>,
    /// One manifest service per connection (keyed by the reader's address).
    manifests: Mutex<HashMap<ClusterId, (usize, HelmService)>>,
    revealed: Mutex<HashSet<ReleaseRef>>,
    reveal_epoch: watch::Sender<u64>,
}

/// The Helm releases of the clusters the app has open. Cheap to clone. See the
/// [module docs](super).
#[derive(Clone)]
pub struct ReleasesService {
    inner: Arc<Inner>,
}

fn disconnected(cluster: &ClusterId) -> OxiError {
    OxiError::network(format!("cluster {cluster} is not connected"))
}

impl ReleasesService {
    /// A service over `source`.
    pub fn new(source: Arc<dyn ReleaseSource>) -> Self {
        Self {
            inner: Arc::new(Inner {
                source,
                manifests: Mutex::new(HashMap::new()),
                revealed: Mutex::new(HashSet::new()),
                reveal_epoch: watch::channel(0).0,
            }),
        }
    }

    fn helm(&self, cluster: &ClusterId) -> OxiResult<Arc<dyn HelmPort>> {
        self.inner
            .source
            .helm(cluster)
            .ok_or_else(|| disconnected(cluster))
    }

    /// The latest revision of every release in the namespaces the cluster's tab watches, by
    /// namespace and name. List-shaped (no values, no notes).
    ///
    /// # Errors
    ///
    /// `Network` while the cluster is not connected; `Forbidden` when RBAC denies listing the
    /// release records; the reader's other errors.
    pub async fn list(&self, cluster: &ClusterId) -> OxiResult<Vec<Release>> {
        let helm = self.helm(cluster)?;
        let namespaces = self.inner.source.namespaces(cluster);
        let mut releases = helm.list_releases(cluster, &namespaces).await?;
        releases.sort_by(|a, b| {
            (&a.release.namespace, &a.release.name).cmp(&(&b.release.namespace, &b.release.name))
        });
        Ok(releases)
    }

    /// A stream of `()`, one per change to the cluster's release records or scope (list again);
    /// `None` while the cluster is not connected. See [`ReleaseWatch`].
    pub fn watch(&self, cluster: &ClusterId) -> Option<ReleaseWatch> {
        ReleaseWatch::open(&*self.inner.source, cluster)
    }

    /// Every revision of `release`, newest first.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown release, `Network` while disconnected.
    pub async fn history(&self, release: &ReleaseRef) -> OxiResult<Vec<Revision>> {
        self.helm(&release.cluster)?.history(release).await
    }

    /// The `NOTES.txt` of a revision (the latest when `None`), with secret-bearing text scrubbed.
    ///
    /// # Errors
    ///
    /// As [`history`](Self::history).
    pub async fn notes(
        &self,
        release: &ReleaseRef,
        revision: Option<u32>,
    ) -> OxiResult<Option<Arc<str>>> {
        let notes = self
            .helm(&release.cluster)?
            .notes(release, revision)
            .await?;
        Ok(notes.map(|text| Arc::from(mask_notes(&text))))
    }

    /// The values of a revision (the latest when `None`): the user's, or with `all` the chart
    /// defaults merged in, as YAML with credential-looking values masked. The masking and the
    /// YAML run on a blocking thread.
    ///
    /// # Errors
    ///
    /// As [`history`](Self::history).
    pub async fn values(
        &self,
        release: &ReleaseRef,
        revision: Option<u32>,
        all: bool,
        disclosure: Disclosure,
    ) -> OxiResult<ValuesView> {
        let values = self
            .helm(&release.cluster)?
            .values(release, revision, all)
            .await?;
        let reveal = disclosure == Disclosure::Granted && self.is_revealed(release);
        render_values(values, reveal).await
    }

    /// The rendered manifest of a revision (the latest when `None`); Secret documents are masked
    /// unless the release is revealed and `disclosure` honours it.
    ///
    /// # Errors
    ///
    /// As [`history`](Self::history).
    pub async fn manifest(
        &self,
        release: &ReleaseRef,
        revision: Option<u32>,
        disclosure: Disclosure,
    ) -> OxiResult<ManifestView> {
        let text = self
            .helm(&release.cluster)?
            .manifest(release, revision)
            .await?;
        let reveal = disclosure == Disclosure::Granted && self.is_revealed(release);
        tokio::task::spawn_blocking(move || {
            if reveal {
                ManifestView {
                    text: Arc::from(text),
                    masked: false,
                }
            } else {
                ManifestView {
                    text: Arc::from(crate::helm_install::mask_secrets(&text)),
                    masked: true,
                }
            }
        })
        .await
        .map_err(|e| OxiError::internal(format!("masking the manifest stopped: {e}")))
    }

    /// The release's objects as `ResourceRef`s joined with the live ones, kept current by the
    /// store's feeds while the lease lives (E16-S03). Drop it when the tab closes.
    ///
    /// # Errors
    ///
    /// As [`history`](Self::history), plus the manifest's read errors and discovery's.
    pub async fn resources(
        &self,
        release: &ReleaseRef,
        revision: u32,
        updated: Timestamp,
    ) -> OxiResult<ReleaseResourcesLease> {
        let cluster = &release.cluster;
        let helm = self.helm(cluster)?;
        let store = self
            .inner
            .source
            .store(cluster)
            .ok_or_else(|| disconnected(cluster))?;
        let kinds = self.inner.source.kinds(cluster).await?;
        let service = self.manifests_for(cluster, helm);
        service
            .open(&store, release, revision, updated, &kinds)
            .await
    }

    fn manifests_for(&self, cluster: &ClusterId, helm: Arc<dyn HelmPort>) -> HelmService {
        let key = Arc::as_ptr(&helm).cast::<()>() as usize;
        let mut services = self.inner.manifests.lock();
        match services.get(cluster) {
            Some((at, service)) if *at == key => service.clone(),
            _ => {
                let service = HelmService::new(helm);
                services.insert(cluster.clone(), (key, service.clone()));
                service
            }
        }
    }

    /// Whether the user revealed `release`'s masked values.
    pub fn is_revealed(&self, release: &ReleaseRef) -> bool {
        self.inner.revealed.lock().contains(release)
    }

    /// Reveals (or hides again) `release`'s values and manifest Secrets in the view, and tells
    /// the views through [`reveal_changes`](Self::reveal_changes). Returns how many values the
    /// release masks. Only `helm::RevealValues` calls this.
    ///
    /// # Errors
    ///
    /// As [`values`](Self::values): the count needs the release's computed values.
    pub async fn set_revealed(&self, release: &ReleaseRef, reveal: bool) -> OxiResult<usize> {
        let view = self.values(release, None, true, Disclosure::Masked).await?;
        let changed = {
            let mut revealed = self.inner.revealed.lock();
            if reveal {
                revealed.insert(release.clone())
            } else {
                revealed.remove(release)
            }
        };
        if changed {
            self.inner.reveal_epoch.send_modify(|epoch| *epoch += 1);
        }
        Ok(view.sensitive)
    }

    /// Hides `release` again (the detail closed). A no-op when it was not revealed.
    pub fn conceal(&self, release: &ReleaseRef) {
        if self.inner.revealed.lock().remove(release) {
            self.inner.reveal_epoch.send_modify(|epoch| *epoch += 1);
        }
    }

    /// Changes whenever a reveal was granted or withdrawn: the views read their tab again.
    pub fn reveal_changes(&self) -> watch::Receiver<u64> {
        self.inner.reveal_epoch.subscribe()
    }
}

async fn render_values(values: Values, reveal: bool) -> OxiResult<ValuesView> {
    tokio::task::spawn_blocking(move || {
        let (masked, sensitive) = mask_values(&values);
        let shown = if reveal { &values } else { &masked };
        let text = shown
            .to_yaml()
            .map_err(|e| OxiError::internal(e.to_string()))?;
        Ok(ValuesView {
            text: Arc::from(text),
            sensitive,
            revealed: reveal,
            empty: values.is_empty(),
        })
    })
    .await
    .map_err(|e| OxiError::internal(format!("rendering the values stopped: {e}")))?
}
