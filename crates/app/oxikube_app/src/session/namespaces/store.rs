//! The service's private plumbing: the per-cluster prefs cache, serialised writes, and the
//! debounce tickets.

use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
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

    pub(super) async fn apply_selection(
        &self,
        cluster: &ClusterId,
        selection: NamespaceSelection,
    ) -> OxiResult<NamespaceOutcome> {
        let session_changed = self
            .shared
            .manager
            .set_namespace_selection(cluster, selection.clone())?;
        let mut outcome = self
            .mutate(cluster, |prefs| {
                let changed = prefs.selection != selection;
                prefs.selection = selection;
                changed
            })
            .await?;
        outcome.changed |= session_changed;
        Ok(outcome)
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
        let loaded = prefs::read(self.shared.state.as_ref(), cluster).await?;
        self.shared
            .cache
            .lock()
            .entry(cluster.clone())
            .or_insert(loaded);
        Ok(())
    }

    /// Applies `change` to the cached prefs; when it reports a change, stores them.
    pub(super) async fn mutate(
        &self,
        cluster: &ClusterId,
        change: impl FnOnce(&mut NamespacePrefs) -> bool,
    ) -> OxiResult<NamespaceOutcome> {
        self.ensure_loaded(cluster).await?;
        let (changed, prefs) = {
            let mut cache = self.shared.cache.lock();
            let prefs = cache.entry(cluster.clone()).or_default();
            let changed = change(prefs);
            (changed, prefs.clone())
        };
        if changed {
            // Write whatever is newest once it is this write's turn, so concurrent changes
            // never store an older snapshot last.
            let _turn = self.shared.write.lock().await;
            let latest = self.cached(cluster);
            prefs::write(self.shared.state.as_ref(), cluster, &latest).await?;
        }
        Ok(NamespaceOutcome { changed, prefs })
    }
}
