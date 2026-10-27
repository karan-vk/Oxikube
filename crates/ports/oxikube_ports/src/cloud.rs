//! [`CloudDiscoveryPort`]: finding clusters through the cloud providers' CLIs.
//!
//! # Adapter
//!
//! Implemented by `oxikube_cloud`, one adapter per provider (`aws.rs` for EKS, `gcloud.rs`
//! for GKE, `az.rs` for AKS), each driving the CLI the user already has logged in through the
//! crate's shared `CliRunner` (`process.rs`). Oxikube never stores their credentials:
//! discovered clusters carry no secrets, and the kubeconfig entries built for them
//! authenticate only through an exec plugin. Calls spawn processes, so run them off the UI
//! thread (`oxikube_runtime::spawn_kube`); dropping a call's future kills its child process.
//!
//! The data types are plain domain data in [`oxikube_domain::cloud`] (re-exported here), so
//! the discovery cache can store them without depending on this crate.

use async_trait::async_trait;
use oxikube_domain::OxiResult;
pub use oxikube_domain::cloud::{
    CloudDetection, CloudProvider, CloudScope, CloudToolStatus, ClusterStatus, DiscoveredCluster,
    ExecEnvVar, ExecInteractiveMode, ExecPlugin, KubeconfigEntry, SetupHint,
};

/// Discovers one provider's managed clusters through its CLI and builds exec-auth kubeconfig
/// entries for them. The app holds one per provider (`Vec<Arc<dyn CloudDiscoveryPort>>`).
///
/// The expected order is `detect` (is the CLI there and logged in?), then `scopes` (which
/// profiles + regions, projects or subscriptions to look in), then `list_clusters` per scope
/// (fanned out by the caller, bounded by the adapter's runner), then `kubeconfig_entry` for
/// the clusters the user adds.
///
/// # Effects
///
/// Read-only at the provider. Spawns the provider CLI, so run it off the UI thread.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`Auth`](oxikube_domain::ErrorKind::Auth) when the CLI is not logged in (the message says
/// how to log in), [`Forbidden`](oxikube_domain::ErrorKind::Forbidden) when the identity may
/// not list clusters, [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) when the CLI or
/// a required auth plugin is missing (the message names the settings key of its path),
/// [`Validation`](oxikube_domain::ErrorKind::Validation) when given a scope or cluster of
/// another provider, [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) when the provider API is
/// unreachable or the CLI hangs. Messages and every stderr excerpt are redacted.
#[async_trait]
pub trait CloudDiscoveryPort: Send + Sync {
    /// Which provider this adapter discovers. Cheap and constant.
    fn id(&self) -> CloudProvider;

    /// Finds the CLI and reports its path, version, login state and setup hints (a missing
    /// auth plugin, an expired login). A missing CLI is
    /// `Ok` with [`CloudToolStatus::NotInstalled`] and no binary, never an error, so the
    /// catalog can hide the provider quietly.
    async fn detect(&self) -> OxiResult<CloudDetection>;

    /// The scopes to list clusters in: AWS profiles x regions, GCP projects, Azure
    /// subscriptions, as the CLI's configuration knows them. Every scope's provider is
    /// [`id`](Self::id).
    async fn scopes(&self) -> OxiResult<Vec<CloudScope>>;

    /// The clusters visible in `scope`, each carrying `scope` itself. A scope of another
    /// provider is a `Validation` error; an empty scope is `Ok(vec![])`.
    async fn list_clusters(&self, scope: &CloudScope) -> OxiResult<Vec<DiscoveredCluster>>;

    /// The kubeconfig entry that connects to `cluster`: its endpoint and CA, and an exec
    /// plugin (`aws eks get-token`, `gke-gcloud-auth-plugin`, `kubelogin`) with
    /// `interactive_mode` set. Never a static token. May run the CLI to fill an endpoint or
    /// CA the listing lacked, and fails with `Unsupported` (naming the plugin and how to
    /// install it) when the auth plugin is missing. A cluster of another provider is a
    /// `Validation` error.
    async fn kubeconfig_entry(&self, cluster: &DiscoveredCluster) -> OxiResult<KubeconfigEntry>;
}
