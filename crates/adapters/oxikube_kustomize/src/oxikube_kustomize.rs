//! `oxikube_kustomize` — layer: `adapters`.
//!
//! The [`KustomizePort`](oxikube_ports::KustomizePort) adapter (E20-S04): it finds the user's
//! `kustomize` (or `kubectl kustomize`) and builds overlay directories with it. It shells out
//! only; no kustomize library is linked.
//!
//! | Module | Holds |
//! |---|---|
//! | [`config`] | [`KustomizeConfig`]: the `kustomize.path` override, timeouts, where to search |
//! | [`path`] | the login shell's `PATH` and the binary lookup (a GUI launch lacks the user's `PATH`) |
//! | `detect` | standalone `kustomize` first, then `kubectl kustomize`; version parsing |
//! | `run` | one child process: separate stdout and stderr, a deadline, killed on drop |
//! | `build` | the build command line (no `--enable-exec`, `--enable-alpha-plugins` or `--enable-helm`) and failure classification |
//!
//! The crate is its own adapter rather than part of `oxikube_kube` (the issue's suggestion):
//! that crate is built around kube-rs, and adapters cannot share code with each other, so the
//! small process runner is copied from `oxikube_cloud` and `oxikube_helm`.
//!
//! See `docs/ARCHITECTURE.md` for the allowed dependency direction. `cargo xtask lint-deps`
//! enforces it.

mod build;
pub mod config;
mod detect;
pub mod path;
mod port;
mod run;

pub use config::{KustomizeConfig, SearchPath};
pub use port::KustomizeCli;
