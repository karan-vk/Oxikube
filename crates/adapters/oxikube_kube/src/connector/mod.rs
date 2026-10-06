//! [`KubeConnector`]: `ClusterConnectorPort` over the [`ClientPool`] (E06-S12).
//!
//! The session manager (`oxikube_app::session`) asks the connector for one connection per
//! kubeconfig context. The connector assembles what the other modules of this crate provide into
//! the [`ClusterPorts`] bundle: the pooled client, discovery, resource reads and writes, Table
//! feeds, logs, exec, port-forward, metrics and the access review.
//!
//! | Piece | Where |
//! |---|---|
//! | the pooled client, one pool per exec-plugin policy | [`ClientPool`] |
//! | `AccessReviewPort`: RBAC rules review plus the metrics flag from discovery | `access` |
//! | the liveness loop and its [`HealthReporter`] bridge, the watch budget | `connection` |
//!
//! # Contract
//!
//! * `connect` builds (or reuses) the client for the context and returns without a network round
//!   trip of its own beyond the client build: discovery and the capability probe are the
//!   manager's calls through the bundle, so their failures (a 401 from a bad token) are
//!   classified there. A context that is not in the kubeconfig is `NotFound`; a plugin that wants
//!   more interaction than the request's [`ExecInteractivity`] allows is a non-retryable `Auth`.
//! * The liveness loop ([`Liveness`](crate::health::Liveness), `GET /version`) starts with the
//!   connection and reports through the request's [`HealthReporter`]. The manager ignores
//!   signals until the session is `Ready`.
//! * Each connection owns a [`FeedRegistry`] (the watch budget, E04-S13), reachable with
//!   [`KubeConnector::feeds`] for as long as the connection lives: the resource store opens its
//!   feeds there and reads the counters with [`FeedRegistry::stats`]. Dropping the
//!   [`ClusterConnection`] stops the liveness loop and the health bridge; feeds stop when the
//!   registry and every lease on it are gone.
//!
//! The kubeconfig is set at construction. Hot reload of sources (E03-S02) is the wiring's job:
//! after each reload it hands the new loader result to [`KubeConnector::replace_loaded`], which
//! passes it to every pool built so far (each drops only the clients of contexts that changed or
//! vanished) and to every pool built later. Live connections keep the client they connected with.
//!
//! Plain Tokio; nothing blocks. Credentials never leave the pool: errors are classified and
//! redacted by the modules this one calls.

mod access;
mod connection;

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use kube::config::Kubeconfig;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{
    ClusterConnection, ClusterConnectorPort, ClusterPorts, ConnectRequest, ConnectionGuard,
    ExecInteractivity,
};
use parking_lot::Mutex;

use self::access::KubeAccess;
use self::connection::ConnectionState;
use crate::auth::{CredentialRefresh, ExecInteractivePolicy};
use crate::budget::{BudgetConfig, FeedRegistry};
use crate::discovery::KubeDiscovery;
use crate::health::{DEFAULT_RULES_TTL, LivenessConfig, RulesCache};
use crate::kubeconfig::LoadedKubeconfig;
use crate::logs::KubeLogs;
use crate::metrics::KubeMetrics;
use crate::pool::{ClientPool, PoolConfig};
use crate::remote::exec::KubeExec;
use crate::remote::portforward::KubePortForward;
use crate::resources::KubeResources;
use crate::warnings::WarningHub;

/// Tuning for a [`KubeConnector`].
#[derive(Debug, Clone)]
pub struct ConnectorConfig {
    /// The liveness loop of every connection.
    pub liveness: LivenessConfig,
    /// The watch budget of every connection.
    pub budget: BudgetConfig,
    /// How long a rules review is cached.
    pub rules_ttl: std::time::Duration,
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            liveness: LivenessConfig::default(),
            budget: BudgetConfig::default(),
            rules_ttl: DEFAULT_RULES_TTL,
        }
    }
}

type PoolFactory = dyn Fn(ExecInteractivity) -> ClientPool + Send + Sync;

/// Connects kubeconfig contexts for the session manager. See the [module docs](self).
///
/// Cheap to clone; clones share the pools and the live connections.
#[derive(Clone)]
pub struct KubeConnector {
    shared: Arc<Shared>,
}

struct Shared {
    /// Builds the pool for one exec-plugin policy; pools are built lazily and kept.
    pools: Mutex<HashMap<ExecInteractivity, Arc<ClientPool>>>,
    factory: Box<PoolFactory>,
    config: ConnectorConfig,
    rules: Arc<RulesCache>,
    /// The live connection of each cluster, weakly: the connection's guard owns the state.
    live: Mutex<HashMap<ClusterId, Weak<ConnectionState>>>,
    /// The last kubeconfig handed to [`KubeConnector::replace_loaded`], for pools built after
    /// it. Locked after `pools`, never before.
    latest: Mutex<Option<Arc<LoadedKubeconfig>>>,
}

impl KubeConnector {
    /// A connector over `kubeconfig`. `pool` carries the client settings; its `exec_policy` is
    /// replaced by the one each connect request asks for.
    pub fn new(kubeconfig: Kubeconfig, pool: PoolConfig, config: ConnectorConfig) -> Self {
        Self::with_pools(
            move |interactivity| {
                let exec_policy = match interactivity {
                    ExecInteractivity::Never => ExecInteractivePolicy::Never,
                    ExecInteractivity::IfAvailable => ExecInteractivePolicy::IfAvailable,
                    ExecInteractivity::Always => ExecInteractivePolicy::Always,
                };
                ClientPool::new(
                    kubeconfig.clone(),
                    PoolConfig {
                        exec_policy,
                        ..pool.clone()
                    },
                )
            },
            config,
        )
    }

    /// A connector that gets its pool for each exec-plugin policy from `factory` (called at
    /// most once per policy). For wiring that builds pools with custom factories and clocks.
    pub fn with_pools(
        factory: impl Fn(ExecInteractivity) -> ClientPool + Send + Sync + 'static,
        config: ConnectorConfig,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                pools: Mutex::new(HashMap::new()),
                factory: Box::new(factory),
                rules: Arc::new(RulesCache::new(config.rules_ttl)),
                config,
                live: Mutex::new(HashMap::new()),
                latest: Mutex::new(None),
            }),
        }
    }

    /// The watch budget of `cluster`'s live connection, or `None` when it is not connected.
    pub fn feeds(&self, cluster: &ClusterId) -> Option<FeedRegistry> {
        let mut live = self.shared.live.lock();
        match live.get(cluster).map(Weak::upgrade) {
            Some(Some(state)) => Some(state.registry.clone()),
            Some(None) => {
                live.remove(cluster);
                None
            }
            None => None,
        }
    }

    /// Hands a reloaded kubeconfig (a [`KubeconfigSources`](crate::sources::KubeconfigSources)
    /// reload) to every pool built so far and to every pool built from now on. Each pool drops
    /// only the clients of contexts whose connection changed or that vanished
    /// ([`ClientPool::replace_loaded`]); connections already made keep their client. Returns the
    /// contexts whose pooled clients were dropped, sorted, without duplicates. The kubeconfig
    /// holds credentials: never log it.
    pub fn replace_loaded(&self, loaded: Arc<LoadedKubeconfig>) -> Vec<ContextName> {
        let pools: Vec<Arc<ClientPool>> = {
            let pools = self.shared.pools.lock();
            *self.shared.latest.lock() = Some(loaded.clone());
            pools.values().cloned().collect()
        };
        let mut dropped: Vec<ContextName> = pools
            .iter()
            .flat_map(|pool| pool.replace_loaded(&loaded))
            .collect();
        dropped.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
        dropped.dedup();
        dropped
    }

    fn pool(&self, interactivity: ExecInteractivity) -> Arc<ClientPool> {
        self.shared
            .pools
            .lock()
            .entry(interactivity)
            .or_insert_with(|| {
                let pool = (self.shared.factory)(interactivity);
                if let Some(loaded) = self.shared.latest.lock().as_ref() {
                    pool.replace_loaded(loaded);
                }
                Arc::new(pool)
            })
            .clone()
    }
}

impl std::fmt::Debug for KubeConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubeConnector")
            .field("pools", &self.shared.pools.lock().len())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ClusterConnectorPort for KubeConnector {
    async fn connect(&self, request: ConnectRequest) -> OxiResult<ClusterConnection> {
        let pool = self.pool(request.exec_interactivity);
        let client = (*pool.get(&request.context).await?).clone();

        let discovery = KubeDiscovery::new(client.clone());
        let resources = KubeResources::new(client.clone(), discovery.clone());
        let access = KubeAccess::new(
            pool.clone(),
            self.shared.rules.clone(),
            request.context.clone(),
            discovery.clone(),
        );
        let registry = FeedRegistry::for_resources(
            request.cluster.clone(),
            resources.clone(),
            self.shared.config.budget.clone(),
        );
        let state = Arc::new(ConnectionState::start(
            registry,
            self.shared.config.liveness,
            pool,
            &request,
        ));
        self.shared
            .live
            .lock()
            .insert(request.cluster.clone(), Arc::downgrade(&state));

        let ports = ClusterPorts {
            resources: Arc::new(resources.clone()),
            discovery: Arc::new(discovery),
            tables: Arc::new(resources),
            logs: Arc::new(KubeLogs::new(client.clone())),
            exec: Arc::new(KubeExec::new(client.clone())),
            port_forward: Arc::new(KubePortForward::new(client.clone())),
            metrics: Arc::new(KubeMetrics::new(client, request.cluster.clone())),
            access: Arc::new(access),
            warnings: Arc::new(WarningHub::global().port(&request.context)),
        };
        Ok(ClusterConnection {
            ports,
            guard: ConnectionGuard::new(state),
        })
    }
}

/// Credential refresh knowledge the connector passes to probes: it does not inspect the
/// kubeconfig user, so a 401 counts as retryable once (`CredentialRefresh::Unknown`).
pub(crate) const REFRESH: CredentialRefresh = CredentialRefresh::Unknown;

#[cfg(test)]
mod tests;
