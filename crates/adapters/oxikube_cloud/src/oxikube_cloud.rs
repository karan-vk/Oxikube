//! `oxikube_cloud` — layer: `adapters`.
//!
//! CloudDiscoveryPort adapters that shell out to installed aws/gcloud/az CLIs to list EKS/GKE/AKS clusters and build exec-auth kubeconfig entries.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
