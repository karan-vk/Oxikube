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
//! | [`auth`] | E03-S04 | `kube::Error` classification, exec-plugin interactivity policy, deadline-bounded client build, retry-once helper |
//! | [`feed`] | E04-S02 | [`ReflectorFeed`]: `watcher` + reflector store per (cluster, gvk, scope), coalesced delta batches on a bounded channel, relist diffs, backoff and [`FeedState`] |
//! | [`discovery`] | E03-S06 | `DiscoveryPort` over aggregated discovery, the shared kind registry and the CRD watcher |
//! | [`pool`] | E03-S03 | [`ClientPool`]: one lazily built, shared kube client per context, with invalidation and LRU eviction |
//! | [`logs`] | E04-S08 | [`KubeLogs`]: `LogPort` with reconnect, overlap and dedup, plus multi-container and label-selector fan-in |
//! | [`resources`] | E04-S01 | [`KubeResources`]: `ResourceReader` list/get with pagination, selectors and `resourceVersion` semantics over typed and dynamic `Api`s |
//! | [`metrics`] | E04-S11 | [`KubeMetrics`]: `MetricsPort` on `metrics.k8s.io` via `k8s-metrics`, exact `Quantity` parsing, absence as a visible state |
//! | [`mutate`] | E04-S05 | [`KubeResources`] as a `ResourceWriter`: create, replace, patch (merge, strategic, JSON, server-side apply), dry-run, delete, delete-collection, with conflict and validation detail in the errors |
//! | [`remote`] | E04-S10 | [`KubePortForward`]: `PortForwardPort` over `Api<Pod>::portforward`, local-listener forwards with service-to-pod resolution and a target-gone hook |
//! | [`subresource`] | E04-S06 | scale, status, eviction, ephemeral containers and resize as `ResourcePort` methods; [`ResourcePatch`] builders (rollout restart, cordon, uncordon, cronjob suspend) ported from kdash |
//! | [`sources`] | E03-S02 | `ClusterSourcePort` over kubeconfig files, directories and pasted text, with hot reload |
//! | [`table`] | E04-S04 | `TableFeedPort` on [`KubeResources`]: hand-rolled server Table API list + watch feed with refresh, diffing and plain-JSON fallback |

pub mod auth;
pub mod discovery;
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

#[cfg(test)]
mod fake_api;

pub use discovery::{
    CrdWatch, CrdWatchConfig, DiscoveryConfig, KindChange, KubeDiscovery, Registry, RegistryDiff,
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
pub use remote::portforward::{ForwardHandle, KubePortForward};
pub use resources::{
    AccessPath, KubeResources, ListExpired, ManagedFields, ResourcesConfig, is_list_expired,
};
pub use subresource::{
    EphemeralContainerSpec, EvictionBlocked, RESTARTED_AT_ANNOTATION, ResizeSpec, ResourcePatch,
    ephemeral_container_patch, eviction_blocked, resize_patch,
};
pub use table::TableConfig;
