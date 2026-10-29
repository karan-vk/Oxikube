//! [`KustomizePort`]: build a kustomize overlay with the user's own `kustomize` (E20-S04).
//!
//! The application layer must not spawn processes, so building goes through this port. The
//! adapter (`oxikube_kustomize`) runs `kustomize build <dir>` or, when no standalone binary
//! exists, `kubectl kustomize <dir>`; tests use `FakeKustomizePort`.
//!
//! The vocabulary ([`KustomizeTool`], [`BuildOutput`], [`KustomizeError`]) lives in
//! [`oxikube_domain::kustomize`] and is re-exported here.
//!
//! # Effects
//!
//! A build reads local files and runs a local process; it never touches a cluster and is not a
//! mutation, so it does not go through `MutationGuard`. The built manifests are applied through
//! the apply service, which is.
//!
//! # Cancellation
//!
//! Dropping the future returned by [`build`](KustomizePort::build) cancels the build and kills
//! the child process. A deadline (60 s by default) kills it too.
//!
//! # Sensitivity
//!
//! `secretGenerator` puts Secret values in the output. Callers parse it as manifest text and
//! never log or persist it.

use std::path::Path;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
pub use oxikube_domain::kustomize::{
    BuildOutput, KustomizeError, KustomizeTool, KustomizeToolKind,
};

/// Builds kustomize overlays.
#[async_trait]
pub trait KustomizePort: Send + Sync {
    /// The tool that would build: the standalone `kustomize`, else `kubectl kustomize`. `None`
    /// when neither exists, which hides every kustomize action. The adapter caches the answer;
    /// [`refresh`](Self::refresh) looks again.
    async fn detect(&self) -> Option<KustomizeTool>;

    /// Looks for the tool again (after the user installed it or changed `kustomize.path`).
    async fn refresh(&self) -> Option<KustomizeTool>;

    /// Builds the overlay in `dir` (a directory with a `kustomization.yaml`).
    ///
    /// # Errors
    ///
    /// [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) when no tool exists or the
    /// overlay needs a feature that is kept disabled (exec or alpha plugins, Helm),
    /// [`Validation`](oxikube_domain::ErrorKind::Validation) when kustomize rejects the
    /// overlay (the message carries its stderr), [`Timeout`](oxikube_domain::ErrorKind::Timeout)
    /// past the deadline.
    async fn build(&self, dir: &Path) -> OxiResult<BuildOutput>;
}
