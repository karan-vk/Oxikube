//! [`HelmError`]: why a Helm operation failed, mapped onto the port error taxonomy.

use crate::error::OxiError;

/// A Helm failure. Adapters build these and convert with `OxiError::from`, so every port
/// method still returns `OxiResult`.
///
/// Text carried here (`stderr_summary`, parse messages) must already be redacted: values
/// and manifests can contain secrets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HelmError {
    /// The adapter does not provide this operation (native-only mode, or no binary).
    #[error("this Helm operation is not available here")]
    Unsupported,
    /// No `helm` binary was found.
    #[error(
        "the helm binary was not found; install Helm 3.10 or newer or set its path in settings"
    )]
    BinaryMissing,
    /// The binary is older than the supported minimum.
    #[error("helm {found} is too old; version {needed} or newer is required")]
    VersionTooOld {
        /// What the binary reported.
        found: String,
        /// The minimum supported version.
        needed: String,
    },
    /// No such release (or revision).
    #[error("release not found: {0}")]
    ReleaseNotFound(String),
    /// The cluster denied access to release data.
    #[error("not permitted to access Helm releases: {0}")]
    Forbidden(String),
    /// The operation outlived its deadline.
    #[error("helm timed out after {0}")]
    Timeout(String),
    /// The `helm` process failed.
    #[error("helm failed (exit code {exit_code:?}): {stderr_summary}")]
    CliFailed {
        /// The exit code, `None` when killed by a signal.
        exit_code: Option<i32>,
        /// A short, redacted summary of stderr.
        stderr_summary: String,
    },
    /// Output or a release record could not be understood.
    #[error("could not parse Helm data: {0}")]
    Parse(String),
}

impl From<HelmError> for OxiError {
    fn from(error: HelmError) -> Self {
        let message = error.to_string();
        match error {
            HelmError::Unsupported | HelmError::BinaryMissing | HelmError::VersionTooOld { .. } => {
                OxiError::unsupported(message)
            }
            HelmError::ReleaseNotFound(_) => OxiError::not_found(message),
            HelmError::Forbidden(_) => OxiError::forbidden(message),
            HelmError::Timeout(_) => OxiError::timeout(message),
            HelmError::CliFailed { .. } => OxiError::internal(message),
            HelmError::Parse(_) => OxiError::validation(message),
        }
    }
}
