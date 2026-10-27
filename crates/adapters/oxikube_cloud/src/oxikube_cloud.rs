//! `oxikube_cloud` — layer: `adapters`.
//!
//! `CloudDiscoveryPort` adapters that shell out to the installed `aws`, `gcloud` and `az` CLIs
//! to list EKS, GKE and AKS clusters and build exec-auth kubeconfig entries (E18).
//!
//! | Module | Holds |
//! |---|---|
//! | [`process`] | [`CliRunner`](process::CliRunner), the one way this crate runs a CLI: timeouts, cancellation, JSON output, a concurrency cap, the login shell's `PATH`; the [`ProcessRunner`](process::ProcessRunner) seam the adapters are tested through |
//! | `testing` | `RecordedRunner`: a [`ProcessRunner`](process::ProcessRunner) answering from recorded output (feature `testing`, and this crate's own tests) |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod process;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
