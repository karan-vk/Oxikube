//! `oxikube_cloud` — layer: `adapters`.
//!
//! `CloudDiscoveryPort` adapters that shell out to the installed `aws`, `gcloud` and `az` CLIs
//! to list EKS, GKE and AKS clusters and build exec-auth kubeconfig entries (E18).
//!
//! | Module | Holds |
//! |---|---|
//! | [`az`] | [`AzureAksAdapter`](az::AzureAksAdapter): AKS through `az` per subscription, `kubelogin` exec entries for Azure AD clusters (E18-S04) |
//! | [`process`] | [`CliRunner`](process::CliRunner), the one way this crate runs a CLI: timeouts, cancellation, JSON output, a concurrency cap, the login shell's `PATH`; the [`ProcessRunner`](process::ProcessRunner) seam the adapters are tested through |
//! | [`aws`] | [`AwsDiscovery`](aws::AwsDiscovery): EKS discovery through the `aws` CLI (E18-S02) |
//! | [`gcloud`] | [`GcloudDiscovery`](gcloud::GcloudDiscovery), the GKE `CloudDiscoveryPort` over `gcloud` and `gke-gcloud-auth-plugin` |
//! | `testing` | `RecordedRunner`: a [`ProcessRunner`](process::ProcessRunner) answering from recorded output (feature `testing`, and this crate's own tests) |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod aws;
pub mod az;
pub mod gcloud;
pub mod process;
mod shared;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
