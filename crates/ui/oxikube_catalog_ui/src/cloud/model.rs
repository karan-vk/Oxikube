//! [`CloudModel`]: what the Cloud section shows. Plain Rust, no GPUI: discovery events in, the
//! provider cards, scope filters and cluster rows out.

use std::collections::HashMap;

use jiff::Timestamp;
use oxikube_app::cloud::{CloudDiscoveryEvent, ManagedEntry};
use oxikube_domain::OxiError;
use oxikube_domain::cloud::{
    CloudDetection, CloudProvider, CloudScope, CloudToolStatus, DiscoveredCluster, SetupHint,
};
use oxikube_domain::ids::ContextName;

use super::remediation::{Issue, Remediation, remediation};

/// Where one provider's discovery stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Nothing known yet (before the first answer).
    Idle,
    /// A run is in progress; `done` of `total` scopes are listed.
    Loading {
        /// Scopes finished.
        done: usize,
        /// Scopes in the run (0 until the run has listed them).
        total: usize,
    },
    /// Listed.
    Loaded,
    /// The CLI works but something is missing (a login, the auth plugin): the hints say what.
    Degraded,
    /// The run failed.
    Error,
}

/// One scope that could not be listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeFailure {
    /// The scope.
    pub scope: CloudScope,
    /// What to do about it.
    pub remediation: Remediation,
}

/// Everything shown for one provider.
#[derive(Debug, Clone)]
pub struct ProviderState {
    /// The provider.
    pub provider: CloudProvider,
    /// What detection found, once known.
    pub detection: Option<CloudDetection>,
    running: Option<(usize, usize)>,
    /// Every cluster known, sorted by key.
    pub clusters: Vec<DiscoveredCluster>,
    /// The scopes of the last run that failed.
    pub failures: Vec<ScopeFailure>,
    /// Why the last run failed as a whole.
    pub error: Option<Remediation>,
    /// When the listing was fetched (the cache's age while a refresh runs).
    pub fetched_at: Option<Timestamp>,
    /// The scope filter; `None` shows all scopes.
    pub filter: Option<CloudScope>,
    /// A run of the CLI (not only the cache) happened since the last cache answer.
    listed_now: bool,
}

impl ProviderState {
    fn new(provider: CloudProvider) -> Self {
        Self {
            provider,
            detection: None,
            running: None,
            clusters: Vec::new(),
            failures: Vec::new(),
            error: None,
            fetched_at: None,
            filter: None,
            listed_now: false,
        }
    }

    /// Whether the provider is shown: its CLI was found. Undetected providers are hidden.
    pub fn is_visible(&self) -> bool {
        self.detection
            .as_ref()
            .is_some_and(|d| d.status != CloudToolStatus::NotInstalled)
    }

    /// Whether a run is in progress.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// The setup hints of the detection (log in, install a plugin).
    pub fn hints(&self) -> &[SetupHint] {
        self.detection.as_ref().map_or(&[], |d| d.hints.as_slice())
    }

    /// The state to show.
    pub fn phase(&self) -> Phase {
        if let Some((done, total)) = self.running {
            Phase::Loading { done, total }
        } else if self.error.is_some() {
            Phase::Error
        } else if !self.hints().is_empty() {
            Phase::Degraded
        } else if self.detection.is_some() {
            Phase::Loaded
        } else {
            Phase::Idle
        }
    }

    /// The distinct scopes of the clusters, in order: the filter bar's choices.
    pub fn scopes(&self) -> Vec<CloudScope> {
        let mut scopes: Vec<CloudScope> = self.clusters.iter().map(|c| c.scope.clone()).collect();
        scopes.sort();
        scopes.dedup();
        scopes
    }

    /// The clusters the filter lets through.
    pub fn shown(&self) -> impl Iterator<Item = &DiscoveredCluster> {
        self.clusters
            .iter()
            .filter(|c| self.filter.as_ref().is_none_or(|f| *f == c.scope))
    }

    fn set_clusters(&mut self, mut clusters: Vec<DiscoveredCluster>) {
        clusters.sort_by_key(DiscoveredCluster::key);
        self.clusters = clusters;
        if let Some(filter) = &self.filter
            && !self.clusters.iter().any(|c| c.scope == *filter)
        {
            self.filter = None;
        }
    }
}

/// The Cloud section's state: one [`ProviderState`] per provider, the selected one, and which
/// clusters are already in the catalog.
#[derive(Debug)]
pub struct CloudModel {
    providers: [ProviderState; 3],
    selected: Option<CloudProvider>,
    /// Discovered-cluster key to the context it was added as.
    added: HashMap<CloudProvider, HashMap<String, ContextName>>,
}

impl CloudModel {
    /// A model with every provider unknown (so hidden).
    pub fn new() -> Self {
        Self {
            providers: CloudProvider::ALL.map(ProviderState::new),
            selected: None,
            added: HashMap::new(),
        }
    }

    /// The state of `provider`.
    pub fn provider(&self, provider: CloudProvider) -> &ProviderState {
        &self.providers[slot(provider)]
    }

    fn provider_mut(&mut self, provider: CloudProvider) -> &mut ProviderState {
        &mut self.providers[slot(provider)]
    }

    /// The providers whose CLI was found, in display order.
    pub fn visible(&self) -> impl Iterator<Item = &ProviderState> {
        self.providers.iter().filter(|p| p.is_visible())
    }

    /// The provider whose clusters are listed: the chosen one, else the first visible one.
    pub fn selected(&self) -> Option<&ProviderState> {
        self.selected
            .map(|p| self.provider(p))
            .filter(|p| p.is_visible())
            .or_else(|| self.visible().next())
    }

    /// Chooses the provider to list.
    pub fn select(&mut self, provider: CloudProvider) {
        self.selected = Some(provider);
    }

    /// Narrows `provider`'s list to one scope (`None`: all).
    pub fn set_filter(&mut self, provider: CloudProvider, filter: Option<CloudScope>) {
        self.provider_mut(provider).filter = filter;
    }

    /// Whether any provider has a run in progress.
    pub fn any_running(&self) -> bool {
        self.providers.iter().any(ProviderState::is_running)
    }

    /// Marks `provider` as loading before its first event arrives, so a second click while the
    /// run starts is ignored. Returns `false` (and changes nothing) when it was running already.
    pub fn begin(&mut self, provider: CloudProvider) -> bool {
        let state = self.provider_mut(provider);
        if state.running.is_some() {
            return false;
        }
        state.running = Some((0, 0));
        state.error = None;
        true
    }

    /// Marks `provider` as no longer running (its command ended).
    pub fn end(&mut self, provider: CloudProvider) -> bool {
        self.provider_mut(provider).running.take().is_some()
    }

    /// Records the clusters in the managed kubeconfig of `provider`.
    pub fn set_added(&mut self, provider: CloudProvider, entries: &[ManagedEntry]) {
        let keyed = entries
            .iter()
            .filter_map(|e| Some((e.cluster_key.clone()?, e.entry.context.clone())))
            .collect();
        self.added.insert(provider, keyed);
    }

    /// The context `cluster` was added as, when it is in the catalog.
    pub fn added_as(&self, cluster: &DiscoveredCluster) -> Option<&ContextName> {
        self.added.get(&cluster.provider())?.get(&cluster.key())
    }

    /// Applies one discovery event; `now` stamps a finished listing.
    pub fn apply(&mut self, event: &CloudDiscoveryEvent, now: Timestamp) {
        match event {
            CloudDiscoveryEvent::Cached {
                provider,
                detection,
                clusters,
                fetched_at,
                stale,
            } => {
                let state = self.provider_mut(*provider);
                if detection.is_some() {
                    state.detection.clone_from(detection);
                }
                state.set_clusters(clusters.clone());
                state.fetched_at = *fetched_at;
                state.listed_now = false;
                if *stale {
                    state.running.get_or_insert((0, 0));
                }
            }
            CloudDiscoveryEvent::Detected(detection) => {
                let state = self.provider_mut(detection.provider);
                state.detection = Some(detection.clone());
                if detection.status == CloudToolStatus::NotInstalled {
                    state.clusters.clear();
                }
            }
            CloudDiscoveryEvent::Started { provider, scopes } => {
                let state = self.provider_mut(*provider);
                state.running = Some((0, scopes.len()));
                state.failures.clear();
                state.error = None;
                state.listed_now = true;
            }
            CloudDiscoveryEvent::ScopeDone { scope, clusters } => {
                let state = self.provider_mut(scope.provider());
                if let Some((done, _)) = &mut state.running {
                    *done += 1;
                }
                let mut all: Vec<_> = state
                    .clusters
                    .iter()
                    .filter(|c| c.scope != *scope)
                    .cloned()
                    .collect();
                all.extend(clusters.iter().cloned());
                state.set_clusters(all);
            }
            CloudDiscoveryEvent::ScopeFailed { scope, error } => {
                let state = self.provider_mut(scope.provider());
                if let Some((done, _)) = &mut state.running {
                    *done += 1;
                }
                state.failures.push(failure(scope, error));
            }
            CloudDiscoveryEvent::Finished {
                provider, clusters, ..
            } => {
                let state = self.provider_mut(*provider);
                state.running = None;
                state.set_clusters(clusters.clone());
                if state.listed_now {
                    state.fetched_at = Some(now);
                }
                state.listed_now = false;
            }
            CloudDiscoveryEvent::Failed { provider, error } => {
                let state = self.provider_mut(*provider);
                state.running = None;
                state.error = Some(remediation(*provider, Issue::of(error), None));
            }
            CloudDiscoveryEvent::Cancelled { provider } => {
                self.provider_mut(*provider).running = None;
            }
        }
    }
}

impl Default for CloudModel {
    fn default() -> Self {
        Self::new()
    }
}

/// The place of `provider` in [`CloudProvider::ALL`], which is the order of the model's array.
fn slot(provider: CloudProvider) -> usize {
    match provider {
        CloudProvider::Aws => 0,
        CloudProvider::Gcp => 1,
        CloudProvider::Azure => 2,
    }
}

fn failure(scope: &CloudScope, error: &OxiError) -> ScopeFailure {
    ScopeFailure {
        scope: scope.clone(),
        remediation: remediation(scope.provider(), Issue::of(error), Some(scope)),
    }
}
