//! `oxikube_helm` — layer: `adapters`.
//!
//! HelmPort adapters: native release Secret/ConfigMap decoder (read-only) and the helm CLI runner for repos/charts/install/upgrade/rollback/uninstall.
//!
//! | Module | Holds |
//! |---|---|
//! | [`cli`] | [`HelmCli`]: the [`HelmPort`](oxikube_ports::HelmPort) over the user's `helm` binary (detection, version gate, cluster env, `-o json` parsers, streamed install/upgrade, timeouts, kill on drop, error mapping) |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod cli;

pub use cli::{HelmCli, HelmCliConfig, HelmTarget, HelmTimeouts, TargetResolver};
