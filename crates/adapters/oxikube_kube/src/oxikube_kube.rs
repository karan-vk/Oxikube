//! `oxikube_kube` — layer: `adapters`.
//!
//! kube-rs adapter: tolerant kubeconfig loading, ClientPool per context with exec/OIDC auth, aggregated discovery, typed + dynamic APIs, reflector feeds, hand-rolled Table API feed, SSA/dry-run/patch/delete, subresources, kubectl-equivalent algorithms (trigger cronjob, rollout undo, drain), logs with reconnect, exec/attach/port-forward over ws, metrics (k8s-metrics), events.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! # Module map
//!
//! | Module | Story | Role |
//! |---|---|---|
//! | [`kubeconfig`] | E03-S01 | tolerant `KUBECONFIG` splitting, loading and merging with per-context origins and diagnostics |
//! | [`health`] | E03-S05 | liveness probe loop with backoff, RBAC rules review cache, capability reduction, `can_i` |
//! | [`budget`] | E04-S13 | [`FeedRegistry`]: the per-cluster watch budget. Shared, ref-counted feeds with idle teardown, feed and object caps (evict idle, degrade to metadata, refuse), per-namespace feeds for a namespace set, tracing spans and [`FeedStats`](oxikube_ports::FeedStats) counters |
//! | [`auth`] | E03-S04 | `kube::Error` classification, exec-plugin interactivity policy, deadline-bounded client build, retry-once helper |
//! | [`feed`] | E04-S02 | [`ReflectorFeed`]: `watcher` + reflector store per (cluster, gvk, scope), coalesced delta batches on a bounded channel, relist diffs, backoff and [`FeedState`] |
//! | [`events`] | E04-S12 | [`KubeEvents`]: `core/v1` and `events.k8s.io/v1` events merged and de-duplicated into domain `Event`s in a fixed-size ring, with per-object (UID) feeds |
//! | [`discovery`] | E03-S06 | `DiscoveryPort` over aggregated discovery, the shared kind registry and the CRD watcher |
//! | [`pool`] | E03-S03 | [`ClientPool`]: one lazily built, shared kube client per context, with invalidation and LRU eviction |
//! | [`logs`] | E04-S08 | [`KubeLogs`]: `LogPort` with reconnect, overlap and dedup, plus multi-container and label-selector fan-in |
//! | [`resources`] | E04-S01 | [`KubeResources`]: `ResourceReader` list/get with pagination, selectors and `resourceVersion` semantics over typed and dynamic `Api`s |
//! | [`metrics`] | E04-S11 | [`KubeMetrics`]: `MetricsPort` on `metrics.k8s.io` via `k8s-metrics`, exact `Quantity` parsing, absence as a visible state |
//! | [`mutate`] | E04-S05 | [`KubeResources`] as a `ResourceWriter`: create, replace, patch (merge, strategic, JSON, server-side apply), dry-run, delete, delete-collection, with conflict and validation detail in the errors |
//! | [`remote`] | E04-S09, E04-S10, E09-S03 | [`KubeExec`]: `ExecPort` over `Api<Pod>::exec` / `attach` handing out [`KubeStream`] terminal backends, node shells with guaranteed cleanup, ephemeral debug containers; [`KubePortForward`]: `PortForwardPort` over `Api<Pod>::portforward`, local-listener forwards with service-to-pod resolution and a target-gone hook |
//! | [`subresource`] | E04-S06 | scale, status, eviction, ephemeral containers and resize as `ResourcePort` methods; [`ResourcePatch`] builders (rollout restart, cordon, uncordon, cronjob suspend) ported from kdash |
//! | [`algorithms`] | E04-S07 | kubectl-equivalent algorithms over a `ResourcePort`: [`trigger_cronjob`], [`rollout_history`], [`rollout_undo`] and [`drain`] (a progress stream with PodDisruptionBudget retry) |
//! | [`connector`] | E06-S12 | [`KubeConnector`]: `ClusterConnectorPort` over the [`ClientPool`]: the per-connection `ClusterPorts` bundle, the RBAC `AccessReviewPort`, the liveness bridge to the session manager and a per-connection [`FeedRegistry`] |
//! | [`warnings`] | E07-S10 | [`WarningLayer`](warnings::WarningLayer) on the client: the API server's `Warning:` headers, redacted, published per context as `WarningPort` |
//! | [`sources`] | E03-S02 | `ClusterSourcePort` over kubeconfig files, directories and pasted text, with hot reload |
//! | [`table`] | E04-S04 | `TableFeedPort` on [`KubeResources`]: hand-rolled server Table API list + watch feed with refresh, diffing and plain-JSON fallback |

pub mod algorithms;
pub mod auth;
pub mod budget;
pub mod connector;
pub mod discovery;
pub mod events;
pub mod feed;
pub mod health;
pub mod kubeconfig;
pub mod logs;
pub mod metrics;
pub mod mutate;
pub mod pool;
pub mod remote;
pub mod resources;
pub mod sources;
pub mod subresource;
pub mod table;
pub mod warnings;

#[cfg(test)]
mod fake_api;

pub use algorithms::{
    BlockReason, DrainOptions, DrainProgress, DrainSummary, PodRef, RolloutRevision, RolloutUndo,
    SkipReason, drain, drain_to_completion, job_from_cronjob, plan_drain, rollout_history,
    rollout_undo, trigger_cronjob,
};
pub use budget::{
    BudgetConfig, ByteCounter, FeedLease, FeedRegistry, FeedRequest, FeedSource, FeedStream,
    ScopeChange, SelectionLease,
};
pub use connector::{ConnectorConfig, DescribeConnection, DescribeFactory, KubeConnector};
pub use discovery::{
    CrdWatch, CrdWatchConfig, DiscoveryConfig, KindChange, KubeDiscovery, Registry, RegistryDiff,
};
pub use events::{
    DEFAULT_EVENT_CAPACITY, EventApi, EventApis, EventFeed, EventFeedStats, EventsConfig,
    EventsOptions, KubeEvents,
};
pub use feed::{
    FeedConfig, FeedKey, FeedObject, FeedState, ReflectorFeed, RelistDelivery, StreamingLists,
};
pub use logs::{ContainerSelection, KubeLogs, LogsConfig};
pub use metrics::KubeMetrics;
pub use mutate::DEFAULT_FIELD_MANAGER;
pub use pool::{
    ClientFactory, ClientPool, Clock, ContextDefinition, EvictionPolicy, KubeClientFactory,
    PoolConfig, ProxyEnv, RetryMode, SystemClock,
};
pub use remote::exec::{DEFAULT_DEBUG_START_TIMEOUT, KubeExec, KubeStream, NodeShellSession};
pub use remote::portforward::{ForwardHandle, KubePortForward};
pub use resources::{
    AccessPath, KubeResources, ListExpired, ManagedFields, ResourcesConfig, is_list_expired,
};
pub use subresource::{
    EphemeralContainerSpec, EvictionBlocked, RESTARTED_AT_ANNOTATION, ResizeSpec, ResourcePatch,
    ephemeral_container_patch, eviction_blocked, resize_patch,
};
pub use table::TableConfig;
