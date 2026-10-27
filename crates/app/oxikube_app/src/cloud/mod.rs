//! Cloud discovery (E18-S05): [`CloudDiscoveryService`] runs the `CloudDiscoveryPort` adapters
//! off the UI thread, caches what they list, and writes the clusters the user adds into
//! Oxikube's own kubeconfig files, which the catalog reads as kubeconfig sources.
//!
//! | File | Holds |
//! |---|---|
//! | `service` | [`CloudDiscoveryService`]: `refresh`, `cancel`, `cached`, `added`, `add`, `remove` |
//! | `run` | one refresh as a task: cache answer, `detect`, the scope fan-out, the terminal event |
//! | `event` | [`CloudDiscoveryEvent`], [`RefreshMode`], [`DiscoveryConfig`], [`AddedCluster`], [`RemovedCluster`] |
//! | `cache` | the per-scope listings and per-provider detections in the state db, with their time |
//! | `managed` | the managed kubeconfigs (`<config dir>/kubeconfigs/<cli>.yaml`) and their YAML |
//!
//! # Runs
//!
//! [`refresh`](CloudDiscoveryService::refresh) spawns one task per provider on the app's
//! [`Spawner`](crate::store::Spawner) (the Tokio bridge `spawn_kube` uses) and returns a stream
//! of [`CloudDiscoveryEvent`]s. The service keeps the task's abort-on-drop guard: a new refresh
//! of the same provider, [`cancel`](CloudDiscoveryService::cancel) or dropping the service
//! aborts it, which drops the listings in flight and so kills their CLI processes. Scopes are
//! listed [`max_parallel_scopes`](DiscoveryConfig::max_parallel_scopes) at a time; one failing
//! scope (not logged in to that profile, forbidden in that project) is reported and the others
//! go on. A provider whose CLI is not installed ends with an empty run (the catalog hides it); a
//! missing auth plugin is a hint on the detection and the provider stays.
//!
//! # Cache
//!
//! Stale-while-revalidate over `StatePort`: a cache within the TTL answers without running the
//! CLI; an older one is sent first and the CLI runs after it. Only `DiscoveredCluster` data and
//! detections are stored (no token: the adapters never produce one).
//!
//! # Managed kubeconfigs
//!
//! [`add`](CloudDiscoveryService::add) merges the provider's exec-auth entry into
//! `<config dir>/kubeconfigs/<cli>.yaml` by context name (adding twice changes nothing), writes
//! it atomically and owner-only through `FsPort::write_private`, and lists the file as a
//! kubeconfig source through [`KubeconfigSourcesService`](crate::KubeconfigSourcesService). The
//! cluster source (E03-S02) reads it with the tolerant loader and watches its directory, so the
//! catalog hot-reloads; the service also reloads at once. The file is a plain file source
//! (`SourceKind::KubeconfigFile`): the sources screen lists it and removing it there deletes
//! it, like a pasted kubeconfig. `SourceKind::Cloud` stays unused, so the adapter needs no
//! change. [`remove`](CloudDiscoveryService::remove) drops the entry (the file and its source
//! entry with the last one), reloads, and disconnects the cluster's open session. The user's
//! `~/.kube/config` is never read or written here.
//!
//! Writing a local kubeconfig is not a cluster mutation, so `MutationGuard` does not apply.

mod cache;
mod event;
mod managed;
mod run;
mod service;

#[cfg(test)]
mod tests;

pub use event::{AddedCluster, CloudDiscoveryEvent, DiscoveryConfig, RefreshMode, RemovedCluster};
pub use managed::ManagedEntry;
pub use service::{CloudDiscoveryService, DiscoveryDeps};
