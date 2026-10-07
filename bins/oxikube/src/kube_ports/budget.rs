//! [`WatchBudgets`]: the kube adapter's per-connection watch budget (`FeedRegistry`), set from
//! the per-cluster `watch_budget` setting and put behind the resource stores' budget hook
//! (E04-F543).
//!
//! Every feed the app opens goes through the connection's `FeedRegistry`: the connector hands
//! out ports whose `watch` and `table_feed` are feeds of it (`oxikube_kube::BudgetedResources`).
//! This module does the rest of the wiring:
//!
//! * limits: each connection starts with its cluster's `watch_budget`
//!   ([`KubeConnector::set_budget_for`]), and [`WatchBudgets::apply`] changes them on live
//!   connections after a settings change (hot reload, no reconnect);
//! * the resource store asks the registry before it opens a feed and the registry holds the
//!   slot until the port call opens it ([`RegistryBudget`], the store's `FeedBudget`): a
//!   refusal makes the store close its own idle feeds first, a full
//!   kind past `metadata_above` is opened metadata-only, a released feed stops counting at once,
//!   and the store's idle grace is the budget's;
//! * counters: [`WatchBudgets::stats`] snapshots every live connection's `FeedStats` (printed by
//!   `oxikube --perf` on exit, [`report_line`]).

use std::sync::Arc;

use oxikube_app::store::{
    Admission, FeedBudget, FeedKind, FeedRequest as StoreRequest, OptionsFor, StoreOptions,
    UnlimitedBudget,
};
use oxikube_domain::ids::ClusterId;
use oxikube_kube::{BudgetConfig, FeedRegistry, FeedRequest, KubeConnector, Verdict};
use oxikube_ports::{ClusterPrefs, ClusterPrefsTable, FeedStats, FeedVariant, WatchBudgetPrefs};
use parking_lot::RwLock;

/// The adapter's limits for a cluster's `watch_budget` setting.
fn budget_config(prefs: &WatchBudgetPrefs) -> BudgetConfig {
    BudgetConfig {
        max_feeds: prefs.max_feeds,
        max_objects: prefs.max_objects,
        metadata_above: prefs.metadata_above,
        idle_grace: prefs.idle_grace,
    }
}

/// The watch budgets of the app's connections. See the [module docs](self).
///
/// Cheap to clone. [`WatchBudgets::disabled`] (the test bundle over fake connectors) has no
/// registries: stores open feeds unlimited and there are no counters.
#[derive(Clone, Default)]
pub struct WatchBudgets {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    kube: KubeConnector,
    /// The per-cluster settings last applied; read at every connect.
    prefs: Arc<RwLock<ClusterPrefsTable>>,
}

impl std::fmt::Debug for WatchBudgets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchBudgets")
            .field("enabled", &self.inner.is_some())
            .finish_non_exhaustive()
    }
}

impl WatchBudgets {
    /// The budgets of `kube`'s connections: every connection made from now on starts with its
    /// cluster's `watch_budget` as last [`apply`](Self::apply)'d (the defaults until then).
    pub fn new(kube: &KubeConnector) -> Self {
        let prefs: Arc<RwLock<ClusterPrefsTable>> = Arc::default();
        let read = prefs.clone();
        kube.set_budget_for(Arc::new(move |cluster: &ClusterId| {
            budget_config(&read.read().get(cluster).watch_budget)
        }));
        Self {
            inner: Some(Arc::new(Inner {
                kube: kube.clone(),
                prefs,
            })),
        }
    }

    /// No budgets: for bundles whose connector is not the kube adapter.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Takes the per-cluster settings `table`: new connections start from it, and every live
    /// connection whose `watch_budget` changed gets the new limits now. Cheap: a lock per
    /// connection, no I/O, safe on the UI thread.
    pub fn apply(&self, table: ClusterPrefsTable) {
        let Some(inner) = &self.inner else { return };
        // The table first: a connection made from now on reads it.
        *inner.prefs.write() = table;
        let table = inner.prefs.read();
        for registry in inner.kube.registries() {
            let config = budget_config(&table.get(registry.cluster()).watch_budget);
            if registry.config() != config {
                tracing::debug!(cluster = %registry.cluster(), "watch budget changed");
                registry.set_config(config);
            }
        }
    }

    /// The resource stores' options: each store asks its connection's budget.
    pub fn store_options(&self) -> Arc<OptionsFor> {
        let budgets = self.clone();
        Arc::new(move |cluster: &ClusterId, _: &ClusterPrefs| StoreOptions {
            budget: match budgets.registry(cluster) {
                Some(registry) => Arc::new(RegistryBudget::new(registry)),
                None => Arc::new(UnlimitedBudget),
            },
            ..StoreOptions::default()
        })
    }

    /// The live connection's budget of `cluster`.
    pub fn registry(&self, cluster: &ClusterId) -> Option<FeedRegistry> {
        self.inner.as_ref()?.kube.feeds(cluster)
    }

    /// The counters of every live connection, sorted by cluster.
    pub fn stats(&self) -> Vec<FeedStats> {
        let Some(inner) = &self.inner else {
            return Vec::new();
        };
        let mut out: Vec<FeedStats> = inner
            .kube
            .registries()
            .iter()
            .map(FeedRegistry::stats)
            .collect();
        out.sort_by(|a, b| a.cluster.cmp(&b.cluster));
        out
    }
}

/// One line of `oxikube --perf` output for a cluster's counters: kinds and numbers only.
pub fn report_line(name: &str, stats: &FeedStats) -> String {
    format!(
        "watch budget `{name}`: feeds {}/{} ({} idle), objects {}/{} (metadata-only above {}), \
         events {}, restarts {}, {} KiB received, errors {}; started {}, stopped {}, \
         degraded {}, refused {}, evicted {}",
        stats.feeds,
        stats.max_feeds,
        stats.idle_feeds,
        stats.objects,
        stats.max_objects,
        stats.metadata_above,
        stats.events,
        stats.restarts,
        stats.bytes / 1024,
        stats.errors,
        stats.started,
        stats.stopped,
        stats.degraded,
        stats.refused,
        stats.evicted,
    )
}

/// The resource store's `FeedBudget` over one connection's `FeedRegistry`.
///
/// The store's feeds are owned feeds of the registry (one per store entry). Before the store
/// opens one it asks [`FeedRegistry::reserve_owned`], which holds the slot from that moment:
/// the store admits synchronously but its driver reaches the port (`open_owned`, which takes
/// the slot) later on a task, so a burst of admissions (one subscribe of several scope parts,
/// several views at once) must each count against the limit before any of them opens, or the
/// surplus would pass here and be refused at the port without the store evicting anything.
/// A refusal makes the store close its own idle feeds, oldest first, and ask again; each one it
/// closes (or gives up before its driver opened it) is [released](FeedRegistry::release_owned)
/// at once, so the next admission already has the room. Its final refusals and its degrades are
/// counted in the registry's stats.
struct RegistryBudget {
    registry: FeedRegistry,
}

impl RegistryBudget {
    fn new(registry: FeedRegistry) -> Self {
        Self { registry }
    }
}

/// The registry's name for a store feed: what `BudgetedResources` names the port call.
fn registry_request(request: &StoreRequest) -> FeedRequest {
    let key = &request.key;
    let feed = FeedRequest::new(key.gvk.clone(), request.kind.variant())
        .in_namespace(key.scope.namespace());
    match &key.selector {
        Some(selector) => feed.labels(selector.to_string()),
        None => feed,
    }
}

impl FeedBudget for RegistryBudget {
    fn admit(&self, request: &StoreRequest, _running: usize) -> Admission {
        match self.registry.reserve_owned(&registry_request(request)) {
            Ok(FeedVariant::Metadata) if request.kind == FeedKind::Full => {
                self.registry.record_verdict(Verdict::Degraded);
                Admission::Degraded(FeedKind::Metadata)
            }
            Ok(_) => Admission::Granted,
            Err(refused) => Admission::Refused(refused.message().to_owned()),
        }
    }

    fn released(&self, request: &StoreRequest) {
        self.registry.release_owned(&registry_request(request));
    }

    fn refused(&self, _: &StoreRequest) {
        self.registry.record_verdict(Verdict::Refused);
    }

    fn idle_grace(&self) -> Option<std::time::Duration> {
        Some(self.registry.config().idle_grace)
    }
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
