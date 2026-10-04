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
//! | [`discovery`] | E03-S06 | `DiscoveryPort` over aggregated discovery, the shared kind registry and the CRD watcher |
//! | [`pool`] | E03-S03 | [`ClientPool`]: one lazily built, shared kube client per context, with invalidation and LRU eviction |
//! | [`sources`] | E03-S02 | `ClusterSourcePort` over kubeconfig files, directories and pasted text, with hot reload |

pub mod auth;
pub mod discovery;
pub mod health;
pub mod kubeconfig;
pub mod pool;
pub mod sources;

#[cfg(test)]
mod fake_api;

pub use discovery::{
    CrdWatch, CrdWatchConfig, DiscoveryConfig, KindChange, KubeDiscovery, Registry, RegistryDiff,
};
pub use pool::{
    ClientFactory, ClientPool, Clock, ContextDefinition, EvictionPolicy, KubeClientFactory,
    PoolConfig, ProxyEnv, RetryMode, SystemClock,
};
