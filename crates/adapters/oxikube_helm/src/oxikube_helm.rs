//! `oxikube_helm` — layer: `adapters`.
//!
//! [`HelmPort`](oxikube_ports::helm::HelmPort) adapters:
//!
//! | Module | Adapter |
//! |---|---|
//! | [`cli`] | [`HelmCli`]: the full port over the user's `helm` binary (detection, version gate, cluster env, `-o json` parsers, streamed install/upgrade, timeouts, kill on drop, error mapping) (E16-S04) |
//! | [`native`] | [`NativeHelmReader`]: releases, history, values, manifest and notes decoded from the release Secrets/ConfigMaps; no `helm` binary needed (E16-S02) |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed dependency
//! direction. `cargo xtask lint-deps` enforces it.

pub mod cli;
pub mod native;

pub use cli::{HelmCli, HelmCliConfig, HelmTarget, HelmTimeouts, TargetResolver};
pub use native::NativeHelmReader;
