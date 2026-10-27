//! Cloud discovery vocabulary (E18): the data `CloudDiscoveryPort` moves between the `aws`,
//! `gcloud` and `az` adapters, the `DiscoveryService` and the catalog's Cloud section.
//!
//! Pure data, serde-able so the discovery cache can store it. Nothing here is a credential:
//! a [`DiscoveredCluster`] carries names, endpoints and the cluster's public CA, and a
//! [`KubeconfigEntry`] authenticates only through an [`ExecPlugin`] that asks the provider's
//! CLI for a short-lived token at connect time. There is no token field anywhere, so a static
//! token cannot be written by mistake.
//!
//! | Module | Holds |
//! |---|---|
//! | [`provider`] | [`CloudProvider`], [`CloudToolStatus`], [`CloudDetection`], [`SetupHint`]: which CLI, and whether it can be used |
//! | [`scope`] | [`CloudScope`]: what one listing covers (AWS profile + region, GCP project, Azure subscription) |
//! | [`cluster`] | [`DiscoveredCluster`], [`ClusterStatus`]: a managed cluster as the provider reports it |
//! | [`kubeconfig`] | [`KubeconfigEntry`], [`ExecPlugin`], [`ExecEnvVar`], [`ExecInteractiveMode`]: the exec-auth context that connects to it |

pub mod cluster;
pub mod kubeconfig;
pub mod provider;
pub mod scope;

pub use cluster::{ClusterStatus, DiscoveredCluster};
pub use kubeconfig::{ExecEnvVar, ExecInteractiveMode, ExecPlugin, KubeconfigEntry};
pub use provider::{CloudDetection, CloudProvider, CloudToolStatus, SetupHint};
pub use scope::CloudScope;
