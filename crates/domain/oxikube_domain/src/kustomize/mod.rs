//! Building kustomize overlays: the detected [`KustomizeTool`], the [`KustomizeError`] a build
//! can fail with and the [`BuildOutput`] it returns (E20-S04).
//!
//! The build runs a local program (`kustomize build`, or `kubectl kustomize`), so the
//! `KustomizePort` in `oxikube_ports` hides it; this module holds only the vocabulary.
//! Build output can carry Secret values (`secretGenerator`), so it is manifest text only:
//! never logged, never persisted.

mod error;
mod tool;

pub use error::KustomizeError;
pub use tool::{BuildOutput, KustomizeTool, KustomizeToolKind};

use crate::command::Capabilities;

/// `capabilities` with [`Capabilities::KUSTOMIZE`] following whether a tool was `detected`, so a
/// command that needs it is hidden while neither binary exists.
pub fn apply_to(capabilities: Capabilities, detected: Option<&KustomizeTool>) -> Capabilities {
    let without = capabilities.difference(Capabilities::KUSTOMIZE);
    if detected.is_some() {
        without.union(Capabilities::KUSTOMIZE)
    } else {
        without
    }
}

#[cfg(test)]
mod tests;
