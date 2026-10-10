//! The service's private plumbing: the per-cluster prefs cache, serialised writes, and the
//! debounce tickets.

use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};

use super::prefs::{self, NamespacePrefs};
use super::service::{NamespaceOutcome, NamespaceService};

impl NamespaceService {
    pub(super) fn session(&self, cluster: &ClusterId) -> OxiResult<crate::ClusterSession> {
        self.shared
            .manager
            .get(cluster)
            .ok_or_else(|| OxiError::not_found(format!("no cluster session {cluster}")))
    }

    pub(super) fn next_ticket(&self, cluster: &ClusterId) -> u64 {
        let mut tickets = self.shared.tickets.lock();
        let ticket = tickets.entry(cluster.clone()).or_insert(0);
        *ticket += 1;
        *ticket
    }

    pub(super) fn cached(&self, cluster: &ClusterId) -> NamespacePrefs {
        self.shared
            .cache
            .lock()
            .get(cluster)
            .cloned()
            .unwrap_or_default()
    }

    pub(super) async fn ensure_loaded(&self, cluster: &ClusterId) -> OxiResult<()> {
        if self.shared.cache.lock().contains_key(cluster) {
            return Ok(());
        }
        let found = prefs::read(self.shared.state.as_ref(), cluster).await?;
        let mut cache = self.shared.cache.lock();
        if cache.contains_key(cluster) {
            return Ok(());
        }
        let loaded = match found {
            Some(stored) => {
                self.shared.stored.lock().insert(cluster.clone());
                stored
            }
            // Nothing remembered: the session's own starting selection (the cluster's default
            // namespace, or the kubeconfig's) is the selection, not `All`.
            None => self.unremembered(cluster),
        };
        cache.insert(cluster.clone(), loaded);
        Ok(())
    }

    /// Default prefs whose selection is the open session's current one.
    pub(super) fn unremembered(&self, cluster: &ClusterId) -> NamespacePrefs {
        NamespacePrefs {
            selection: self
                .shared
                .manager
                .get(cluster)
                .map(|session| session.namespace_selection().clone())
                .unwrap_or_default(),
            ..NamespacePrefs::default()
        }
    }

    /// Applies `change` to the cached prefs; when it reports a change, stores them.
    pub(super) async fn mutate(
        &self,
        cluster: &ClusterId,
        change: impl FnOnce(&mut NamespacePrefs) -> bool,
    ) -> OxiResult<NamespaceOutcome> {
        self.mutate_inner(cluster, false, change).await
    }

    /// [`mutate`](Self::mutate), storing the prefs even when `change` reports none if `force`.
    pub(super) async fn mutate_inner(
        &self,
        cluster: &ClusterId,
        force: bool,
        change: impl FnOnce(&mut NamespacePrefs) -> bool,
    ) -> OxiResult<NamespaceOutcome> {
        self.ensure_loaded(cluster).await?;
        let (changed, prefs) = {
            let mut cache = self.shared.cache.lock();
            let prefs = cache.entry(cluster.clone()).or_default();
            let changed = change(prefs);
            (changed, prefs.clone())
        };
        if changed || force {
            // Write whatever is newest once it is this write's turn, so concurrent changes
            // never store an older snapshot last.
            let _turn = self.shared.write.lock().await;
            let latest = self.cached(cluster);
            prefs::write(self.shared.state.as_ref(), cluster, &latest).await?;
            self.shared.stored.lock().insert(cluster.clone());
        }
        Ok(NamespaceOutcome { changed, prefs })
    }
}
