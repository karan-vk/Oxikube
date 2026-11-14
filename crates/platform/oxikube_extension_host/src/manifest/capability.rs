//! [`ExtensionCapability`]: what an extension may ask the host for.

use serde::Deserialize;

/// A capability an extension declares; the user grants it at install time
/// (E23-S04). Patterns are globs, see the [module docs](super).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ExtensionCapability {
    /// Run a host process.
    #[serde(rename = "process:exec")]
    ProcessExec {
        /// Command patterns allowed to run.
        command: String,
        /// Argument patterns allowed (each argument must match one).
        #[serde(default)]
        args: Vec<String>,
    },
    /// Download a file over the network.
    #[serde(rename = "download_file")]
    DownloadFile {
        /// Host pattern, e.g. `github.com`.
        host: String,
        /// URL path pattern, e.g. `/org/repo/*`.
        path: String,
    },
    /// Read from Kubernetes clusters (never mutate).
    #[serde(rename = "kube:read")]
    KubeRead {
        /// Kubeconfig context-name patterns the extension may read.
        contexts: Vec<String>,
    },
}

impl ExtensionCapability {
    /// The `kind` tag as written in `extension.toml`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ProcessExec { .. } => "process:exec",
            Self::DownloadFile { .. } => "download_file",
            Self::KubeRead { .. } => "kube:read",
        }
    }

    /// Every pattern the capability carries, for validation.
    pub(super) fn patterns(&self) -> Vec<&str> {
        match self {
            Self::ProcessExec { command, args } => std::iter::once(command.as_str())
                .chain(args.iter().map(String::as_str))
                .collect(),
            Self::DownloadFile { host, path } => vec![host, path],
            Self::KubeRead { contexts } => contexts.iter().map(String::as_str).collect(),
        }
    }
}
