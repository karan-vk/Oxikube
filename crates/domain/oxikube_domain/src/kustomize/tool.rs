//! [`KustomizeTool`] and [`BuildOutput`].

use std::fmt;
use std::path::PathBuf;

/// Which program builds overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KustomizeToolKind {
    /// The standalone `kustomize` binary (`kustomize build <dir>`).
    Standalone,
    /// The `kustomize` built into `kubectl` (`kubectl kustomize <dir>`).
    Kubectl,
}

impl KustomizeToolKind {
    /// The command as a user types it in a terminal.
    pub const fn command(self) -> &'static str {
        match self {
            Self::Standalone => "kustomize build",
            Self::Kubectl => "kubectl kustomize",
        }
    }
}

/// A usable kustomize: what was found, where, and which version, for the UI and bug reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KustomizeTool {
    /// Standalone or through `kubectl`.
    pub kind: KustomizeToolKind,
    /// The kustomize version (`v5.4.2`), or `unknown` when the tool would not say.
    pub version: String,
    /// The executable that runs the build (`kustomize` or `kubectl`).
    pub path: PathBuf,
}

impl fmt::Display for KustomizeTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.kind.command(), self.version)
    }
}

/// What a successful build printed.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct BuildOutput {
    /// The built manifests (a YAML stream). May contain Secret values: parse it, never log it.
    pub yaml: String,
    /// Warnings kustomize wrote to stderr while succeeding (redacted).
    pub stderr: String,
}

impl fmt::Debug for BuildOutput {
    /// Sizes only: the manifest text may hold Secret values.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuildOutput")
            .field("yaml_bytes", &self.yaml.len())
            .field("stderr", &self.stderr)
            .finish()
    }
}
