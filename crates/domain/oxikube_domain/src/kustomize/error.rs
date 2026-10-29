//! [`KustomizeError`]: why a build failed, mapped onto the port error taxonomy.

use crate::error::OxiError;

/// A failed build. Text carried here is already redacted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KustomizeError {
    /// Neither `kustomize` nor `kubectl kustomize` was found.
    #[error(
        "neither kustomize nor kubectl was found; install one of them or set kustomize.path in settings"
    )]
    NotInstalled,
    /// The build outlived its deadline.
    #[error("kustomize build timed out after {0}")]
    Timeout(String),
    /// The build was cancelled.
    #[error("kustomize build was cancelled")]
    Cancelled,
    /// The build used a feature this app keeps disabled (exec plugins, alpha plugins, Helm).
    #[error(
        "the overlay needs a kustomize feature Oxikube keeps disabled for safety ({feature}); \
         run `{command} --{flag} <dir>` in a terminal and apply the output instead.\n{stderr}"
    )]
    FeatureDisabled {
        /// What the overlay wanted (`exec or alpha plugins`, `the Helm chart inflator`).
        feature: &'static str,
        /// The flag that would enable it.
        flag: &'static str,
        /// The command to run in a terminal.
        command: &'static str,
        /// kustomize's own message (redacted).
        stderr: String,
    },
    /// The build failed.
    #[error("kustomize build failed (exit code {exit_code:?}): {stderr}")]
    Failed {
        /// The exit code, `None` when killed by a signal.
        exit_code: Option<i32>,
        /// kustomize's stderr (redacted, trimmed).
        stderr: String,
    },
    /// The program could not be run.
    #[error("could not run kustomize: {0}")]
    Io(String),
}

impl KustomizeError {
    /// kustomize's stderr, when the error has any: what the result panel shows.
    pub fn stderr(&self) -> Option<&str> {
        match self {
            Self::FeatureDisabled { stderr, .. } | Self::Failed { stderr, .. } => Some(stderr),
            _ => None,
        }
    }
}

impl From<KustomizeError> for OxiError {
    fn from(error: KustomizeError) -> Self {
        let message = error.to_string();
        match error {
            KustomizeError::NotInstalled | KustomizeError::FeatureDisabled { .. } => {
                OxiError::unsupported(message)
            }
            KustomizeError::Timeout(_) => OxiError::timeout(message),
            KustomizeError::Failed { .. } => OxiError::validation(message),
            KustomizeError::Cancelled | KustomizeError::Io(_) => OxiError::internal(message),
        }
    }
}
