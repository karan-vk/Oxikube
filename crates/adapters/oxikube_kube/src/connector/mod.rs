//! [`KubeConnector`]: `ClusterConnectorPort` over the [`ClientPool`] (E06-S12).
//!
//! The session manager (`oxikube_app::session`) asks the connector for one connection per
//! kubeconfig context. The connector assembles what the other modules of this crate provide into
//! the [`ClusterPorts`] bundle: the pooled client, discovery, resource reads and writes, Table
//! feeds, logs, exec, port-forward, metrics, the access review and the OpenAPI schemas
//! (`SchemaPort`, E10-S01; raw documents are cached on disk once [`KubeConnector::set_schema_cache`] is set).
//!
//! | Piece | Where |
//! |---|---|
//! | the pooled client, one pool per exec-plugin policy | [`ClientPool`] |
//! | `AccessReviewPort`: RBAC rules review plus the metrics flag from discovery | `access` |
//! | the liveness loop and its [`HealthReporter`](oxikube_ports::HealthReporter) bridge, the watch budget | `connection` |
//!
//! # Contract
//!
//! * `connect` builds (or reuses) the client for the context and returns without a network round
//!   trip of its own beyond the client build: discovery and the capability probe are the
//!   manager's calls through the bundle, so their failures (a 401 from a bad token) are
//!   classified there. A context that is not in the kubeconfig is `NotFound`; a plugin that wants
//!   more interaction than the request's [`ExecInteractivity`] allows is a non-retryable `Auth`.
//! * The liveness loop ([`Liveness`](crate::health::Liveness), `GET /version`) starts with the
//!   connection and reports through the request's [`HealthReporter`](oxikube_ports::HealthReporter). The manager ignores
//!   signals until the session is `Ready`.
//! * Each connection owns a [`FeedRegistry`] (the watch budget, E04-S13), reachable with
//!   [`KubeConnector::feeds`] for as long as the connection lives. The bundle's `resources` and
//!   `tables` are a [`BudgetedResources`] over it (E04-F543): every `watch` and `table_feed` the
//!   app makes is a feed of that registry, so the limits hold for tables, sidebar counts, detail
//!   views and log targets alike, and [`FeedRegistry::stats`] counts them. Its limits come from
//!   [`KubeConnector::set_budget_for`] (the per-cluster settings, in the app) or else
//!   [`ConnectorConfig::budget`]; the wiring changes them on live connections through
//!   [`KubeConnector::registries`]. Dropping the [`ClusterConnection`] stops the liveness loop
//!   and the health bridge; a feed stops when its consumer drops its stream.
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
mod describe;

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use kube::config::Kubeconfig;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{
    ClusterConnection, ClusterConnectorPort, ClusterPorts, ConnectRequest, ConnectionGuard,
    DescribePort, DiscoveryPort, ExecInteractivity, FsPort,
};
use parking_lot::Mutex;

use self::access::KubeAccess;
use self::connection::ConnectionState;
pub use self::describe::{DescribeConnection, DescribeFactory};
use crate::auth::{CredentialRefresh, ExecInteractivePolicy};
use crate::budget::{BudgetConfig, BudgetedResources, FeedRegistry};
use crate::discovery::KubeDiscovery;
use crate::health::{DEFAULT_RULES_TTL, LivenessConfig, RulesCache};
use crate::kubeconfig::LoadedKubeconfig;
use crate::logs::KubeLogs;
use crate::metrics::KubeMetrics;
use crate::openapi::OpenApiSchemas;
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

/// The watch budget a new connection to a cluster gets (see [`KubeConnector::set_budget_for`]).
pub type BudgetFor = dyn Fn(&ClusterId) -> BudgetConfig + Send + Sync;

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
    /// Builds each connection's `DescribePort` (set by [`KubeConnector::set_describe_factory`]).
    describe: Mutex<Option<Arc<DescribeFactory>>>,
    /// Each new connection's watch budget (set by [`KubeConnector::set_budget_for`]).
    budget: Mutex<Option<Arc<BudgetFor>>>,
    /// Where each connection's `SchemaPort` keeps raw OpenAPI documents (set by
    /// [`KubeConnector::set_schema_cache`]); memory only until then.
    schema_cache: Mutex<Option<(Arc<dyn FsPort>, std::path::PathBuf)>>,
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
                describe: Mutex::new(None),
                budget: Mutex::new(None),
                schema_cache: Mutex::new(None),
            }),
        }
    }

    /// Sets how each new connection gets its `DescribePort`: `factory` is called with the
    /// connection's client and discovery (see [`DescribeConnection`]). Until it is set a
    /// connection's describe port answers `Unsupported`. Live connections keep the port they
    /// were made with.
    pub fn set_describe_factory(&self, factory: Arc<DescribeFactory>) {
        *self.shared.describe.lock() = Some(factory);
    }

    /// Sets where each new connection's `SchemaPort` keeps the raw OpenAPI documents it fetched:
    /// under `dir` through `fs`, keyed by cluster, server version and document hash. Until it is
    /// set schemas are cached in memory only. Live connections keep the port they were made with.
    pub fn set_schema_cache(&self, fs: Arc<dyn FsPort>, dir: std::path::PathBuf) {
        *self.shared.schema_cache.lock() = Some((fs, dir));
    }

    /// Sets the watch budget each new connection starts with: `budget` is called with the
    /// cluster at connect. Until it is set, every connection gets [`ConnectorConfig::budget`].
    /// Live connections keep theirs; change them through [`registries`](Self::registries).
    pub fn set_budget_for(&self, budget: Arc<BudgetFor>) {
        *self.shared.budget.lock() = Some(budget);
    }

    /// The watch budgets of every live connection.
    pub fn registries(&self) -> Vec<FeedRegistry> {
        let mut live = self.shared.live.lock();
        live.retain(|_, state| state.strong_count() > 0);
        live.values()
            .filter_map(Weak::upgrade)
            .map(|state| state.registry.clone())
            .collect()
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

    fn describe_port(
        &self,
        request: &ConnectRequest,
        client: &kube::Client,
        discovery: &Arc<dyn DiscoveryPort>,
    ) -> Arc<dyn DescribePort> {
        let factory = self.shared.describe.lock().clone();
        match factory {
            Some(factory) => factory(DescribeConnection {
                client: client.clone(),
                discovery: discovery.clone(),
                cluster: request.cluster.clone(),
                context: request.context.clone(),
            }),
            None => Arc::new(describe::NoDescribe),
        }
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
        let budget = self.shared.budget.lock().clone();
        let budget = budget.map_or_else(
            || self.shared.config.budget.clone(),
            |budget| budget(&request.cluster),
        );
        let registry =
            FeedRegistry::for_resources(request.cluster.clone(), resources.clone(), budget);
        let budgeted = Arc::new(BudgetedResources::new(resources, registry.clone()));
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

        let discovery: Arc<dyn DiscoveryPort> = Arc::new(discovery);
        let describe = self.describe_port(&request, &client, &discovery);
        let schemas = OpenApiSchemas::new(client.clone(), request.cluster.clone());
        let schemas = match self.shared.schema_cache.lock().clone() {
            Some((fs, dir)) => schemas.with_disk_cache(fs, dir),
            None => schemas,
        };
        let ports = ClusterPorts {
            resources: budgeted.clone(),
            discovery,
            describe,
            tables: budgeted,
            logs: Arc::new(KubeLogs::new(client.clone())),
            exec: Arc::new(KubeExec::new(client.clone())),
            port_forward: Arc::new(KubePortForward::new(client.clone())),
            metrics: Arc::new(KubeMetrics::new(client, request.cluster.clone())),
            access: Arc::new(access),
            warnings: Arc::new(WarningHub::global().port(&request.context)),
            schemas: Arc::new(schemas),
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
