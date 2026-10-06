//! [`NamespaceService`]: the selection, favourites and namespace list of every cluster.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::lock::Mutex as AsyncMutex;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ClockPort, StatePort};
use parking_lot::Mutex;

use super::catalog::{self, NamespaceCatalog, NamespaceSource, is_valid_namespace_name};
use super::prefs::NamespacePrefs;
use crate::session::ClusterSessionManager;

/// How long [`select_debounced`](NamespaceService::select_debounced) waits for the next toggle
/// before it re-scopes the feeds.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(150);

/// Configuration of the [`NamespaceService`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamespaceConfig {
    /// The quiet time [`select_debounced`](NamespaceService::select_debounced) waits for.
    pub debounce: Duration,
}

impl Default for NamespaceConfig {
    fn default() -> Self {
        Self {
            debounce: DEFAULT_DEBOUNCE,
        }
    }
}

/// The result of a change: what is remembered now, and whether anything changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceOutcome {
    /// Whether the session selection or the remembered prefs changed.
    pub changed: bool,
    /// The remembered prefs after the change.
    pub prefs: NamespacePrefs,
}

/// What [`reconcile`](NamespaceService::reconcile) found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// The namespaces on offer.
    pub catalog: NamespaceCatalog,
    /// The remembered prefs after pruning.
    pub prefs: NamespacePrefs,
    /// Selected namespaces that no longer exist and were dropped from the selection. The UI
    /// tells the user with a toast.
    pub dropped: Vec<String>,
}

/// Namespace selection for every cluster: sets it on the session, remembers it, lists the
/// namespaces on offer. See the [module docs](super).
///
/// Cheap to clone; clones share the cache.
#[derive(Clone)]
pub struct NamespaceService {
    pub(super) shared: Arc<Shared>,
}

pub(super) struct Shared {
    pub(super) manager: ClusterSessionManager,
    pub(super) state: Arc<dyn StatePort>,
    pub(super) clock: Arc<dyn ClockPort>,
    pub(super) config: NamespaceConfig,
    pub(super) cache: Mutex<HashMap<ClusterId, NamespacePrefs>>,
    /// The latest debounce ticket per cluster. A pending debounced selection applies only if
    /// its ticket is still the latest when its quiet time ends.
    pub(super) tickets: Mutex<HashMap<ClusterId, u64>>,
    /// Serialises writes so the last one to start is the last one stored.
    pub(super) write: AsyncMutex<()>,
}

impl std::fmt::Debug for NamespaceService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamespaceService")
            .field("clusters", &self.shared.cache.lock().len())
            .finish_non_exhaustive()
    }
}

impl NamespaceService {
    /// A service with the default configuration. `clock` times the debounce.
    pub fn new(
        manager: ClusterSessionManager,
        state: Arc<dyn StatePort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self::with_config(manager, state, clock, NamespaceConfig::default())
    }

    /// A service with `config`.
    pub fn with_config(
        manager: ClusterSessionManager,
        state: Arc<dyn StatePort>,
        clock: Arc<dyn ClockPort>,
        config: NamespaceConfig,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                manager,
                state,
                clock,
                config,
                cache: Mutex::new(HashMap::new()),
                tickets: Mutex::new(HashMap::new()),
                write: AsyncMutex::new(()),
            }),
        }
    }

    /// The session manager this service drives.
    pub fn manager(&self) -> &ClusterSessionManager {
        &self.shared.manager
    }

    /// The configured debounce.
    pub fn debounce(&self) -> Duration {
        self.shared.config.debounce
    }

    /// The remembered prefs of `cluster` (read from `StatePort` the first time).
    ///
    /// # Errors
    ///
    /// The state store's error.
    pub async fn prefs(&self, cluster: &ClusterId) -> OxiResult<NamespacePrefs> {
        self.ensure_loaded(cluster).await?;
        Ok(self.cached(cluster))
    }

    /// Applies the remembered selection to the session: call it when `cluster` connects.
    /// Sends `NamespaceChanged` when the remembered selection differs from the session's.
    ///
    /// # Errors
    ///
    /// `NotFound` when the session is not open; the state store's error.
    pub async fn restore(&self, cluster: &ClusterId) -> OxiResult<NamespacePrefs> {
        let prefs = self.prefs(cluster).await?;
        self.shared
            .manager
            .set_namespace_selection(cluster, prefs.selection.clone())?;
        Ok(prefs)
    }

    /// Lists the namespaces to offer. The cluster's own list when it answers; the typed names
    /// when it answers `Forbidden` or cannot be read at all (the failure is a
    /// [`NamespaceSource`], not an error).
    ///
    /// # Errors
    ///
    /// The state store's error.
    pub async fn catalog(&self, cluster: &ClusterId) -> OxiResult<NamespaceCatalog> {
        let typed = self.prefs(cluster).await?.typed;
        let reader = self
            .shared
            .manager
            .get(cluster)
            .and_then(|session| session.resources());
        let Some(reader) = reader else {
            return Ok(NamespaceCatalog::new(typed, NamespaceSource::Unavailable));
        };
        Ok(match catalog::list_names(reader.as_ref()).await {
            Ok(names) => NamespaceCatalog::new(names, NamespaceSource::Cluster),
            Err(err) => {
                tracing::debug!(%cluster, kind = ?err.kind(), "namespace list failed; using typed names");
                NamespaceCatalog::new(typed, NamespaceCatalog::source_for(err.kind()))
            }
        })
    }

    /// Lists the namespaces and drops selected ones the cluster no longer has. Pruning only
    /// happens against the cluster's own list ([`NamespaceCatalog::is_authoritative`]): a
    /// forbidden or unreadable list proves nothing.
    ///
    /// # Errors
    ///
    /// `NotFound` when the session is not open; the state store's error.
    pub async fn reconcile(&self, cluster: &ClusterId) -> OxiResult<Reconciled> {
        let session = self.session(cluster)?;
        let catalog = self.catalog(cluster).await?;
        let mut dropped = Vec::new();
        if catalog.is_authoritative() {
            let current = session.namespace_selection();
            dropped = current
                .names()
                .filter(|name| !catalog.contains(name))
                .map(str::to_owned)
                .collect();
            if !dropped.is_empty() {
                let kept = current.names().filter(|name| catalog.contains(name));
                self.select(cluster, NamespaceSelection::from_names(kept))
                    .await?;
            }
        }
        let prefs = self.prefs(cluster).await?;
        Ok(Reconciled {
            catalog,
            prefs,
            dropped,
        })
    }

    /// [`restore`](Self::restore) then [`reconcile`](Self::reconcile): what the selector does
    /// when it opens for a connected cluster.
    ///
    /// # Errors
    ///
    /// As the two steps.
    pub async fn start(&self, cluster: &ClusterId) -> OxiResult<Reconciled> {
        self.restore(cluster).await?;
        self.reconcile(cluster).await
    }

    /// Sets the selection now: updates the session (one `NamespaceChanged` when it differs),
    /// remembers it, and cancels a pending [`select_debounced`](Self::select_debounced). An
    /// empty selection is `All` ([`NamespaceSelection::from_names`]).
    ///
    /// # Errors
    ///
    /// `NotFound` when the session is not open; the state store's error.
    pub async fn select(
        &self,
        cluster: &ClusterId,
        selection: NamespaceSelection,
    ) -> OxiResult<NamespaceOutcome> {
        self.next_ticket(cluster);
        self.apply_selection(cluster, selection).await
    }

    /// Like [`select`](Self::select), but waits [`debounce`](Self::debounce) first and does
    /// nothing when a newer `select` or `select_debounced` arrived meanwhile (`None`), so
    /// ticking five boxes re-scopes the feeds once. Run it off the UI thread; dropping the
    /// future cancels the wait.
    ///
    /// # Errors
    ///
    /// As [`select`](Self::select).
    pub async fn select_debounced(
        &self,
        cluster: &ClusterId,
        selection: NamespaceSelection,
    ) -> OxiResult<Option<NamespaceOutcome>> {
        let ticket = self.next_ticket(cluster);
        self.shared.clock.sleep(self.shared.config.debounce).await;
        if self.shared.tickets.lock().get(cluster) != Some(&ticket) {
            return Ok(None);
        }
        self.apply_selection(cluster, selection).await.map(Some)
    }

    /// Pins `namespace`, or unpins it when it is pinned. Whether it is a favourite now is in
    /// `prefs.favourites`.
    ///
    /// # Errors
    ///
    /// `Validation` for a blank name; the state store's error.
    pub async fn toggle_favourite(
        &self,
        cluster: &ClusterId,
        namespace: &str,
    ) -> OxiResult<NamespaceOutcome> {
        if namespace.trim().is_empty() {
            return Err(OxiError::validation("a namespace name must not be blank"));
        }
        self.mutate(cluster, |prefs| {
            prefs.favourites.toggle(namespace);
            true
        })
        .await
    }

    /// Remembers a namespace name the user typed (for clusters that refuse to list them).
    ///
    /// # Errors
    ///
    /// `Validation` when `name` is not a valid namespace name; the state store's error.
    pub async fn add_typed(&self, cluster: &ClusterId, name: &str) -> OxiResult<NamespaceOutcome> {
        let name = name.trim();
        if !is_valid_namespace_name(name) {
            return Err(OxiError::validation(format!(
                "`{name}` is not a valid namespace name"
            )));
        }
        self.mutate(cluster, |prefs| prefs.add_typed(name)).await
    }

    /// Forgets a typed namespace name.
    ///
    /// # Errors
    ///
    /// The state store's error.
    pub async fn remove_typed(
        &self,
        cluster: &ClusterId,
        name: &str,
    ) -> OxiResult<NamespaceOutcome> {
        self.mutate(cluster, |prefs| prefs.remove_typed(name)).await
    }
}
