//! What to tell the user when discovery or adding fails: one table from [`Issue`] to words and,
//! where there is one, the command that fixes it. Pure data, so every row is testable and the
//! view never shows a raw CLI failure where a next step exists.

use oxikube_domain::cloud::{CloudProvider, CloudScope};
use oxikube_domain::{ErrorKind, OxiError};

/// Why a provider could not be used, as far as the user can act on it. The "error code" of the
/// remediation table: derived from the adapter's [`ErrorKind`] (and, for `Unsupported`, which
/// thing is unsupported).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Issue {
    /// The CLI has no usable login (`ErrorKind::Auth`).
    NotLoggedIn,
    /// The CLI is not installed or not at the configured path.
    CliMissing,
    /// The CLI is too old for the command Oxikube runs.
    CliTooOld,
    /// The exec auth plugin (`gke-gcloud-auth-plugin`, `kubelogin`) is missing.
    PluginMissing,
    /// Logged in but not allowed to list clusters (`ErrorKind::Forbidden`).
    Forbidden,
    /// The provider could not be reached (`ErrorKind::Network`).
    Network,
    /// The CLI took too long (`ErrorKind::Timeout`).
    Timeout,
    /// An AWS profile has no region to scan (`ErrorKind::Validation`).
    NoRegions,
    /// Anything else; the adapter's own (redacted) message is the best there is.
    Other,
}

impl Issue {
    /// Every issue, for the table's tests.
    pub const ALL: [Self; 9] = [
        Self::NotLoggedIn,
        Self::CliMissing,
        Self::CliTooOld,
        Self::PluginMissing,
        Self::Forbidden,
        Self::Network,
        Self::Timeout,
        Self::NoRegions,
        Self::Other,
    ];

    /// Classifies an adapter error by its kind and, for `Unsupported`, by what its message says
    /// is missing (the wording is fixed in `oxikube_cloud`).
    pub fn of(error: &OxiError) -> Self {
        match error.kind() {
            ErrorKind::Auth => Self::NotLoggedIn,
            ErrorKind::Forbidden => Self::Forbidden,
            ErrorKind::Network => Self::Network,
            ErrorKind::Timeout => Self::Timeout,
            ErrorKind::Validation
                if error
                    .message()
                    .to_ascii_lowercase()
                    .contains("no aws regions") =>
            {
                Self::NoRegions
            }
            ErrorKind::Unsupported => {
                let message = error.message().to_ascii_lowercase();
                if message.contains("is not installed")
                    && (message.contains("plugin") || message.contains("kubelogin"))
                {
                    Self::PluginMissing
                } else if message.contains("does not support") && message.contains("upgrade") {
                    Self::CliTooOld
                } else if message.contains("was not found on") {
                    Self::CliMissing
                } else {
                    // Disabled API, stopped cluster, local accounts, odd `--version`: the
                    // adapter's own message carries the fix.
                    Self::Other
                }
            }
            _ => Self::Other,
        }
    }
}

/// What the user is told and can copy for an [`Issue`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remediation {
    /// The issue.
    pub issue: Issue,
    /// One or two sentences: what is wrong and what to do.
    pub text: String,
    /// The command to run in a terminal, when there is one.
    pub command: Option<String>,
}

/// The table: `provider` and `issue` to words. `scope` names the AWS profile in the login
/// command when the failing scope is known.
pub fn remediation(
    provider: CloudProvider,
    issue: Issue,
    scope: Option<&CloudScope>,
) -> Remediation {
    let cli = provider.cli();
    let (text, command): (String, Option<String>) = match issue {
        Issue::NotLoggedIn => {
            let command = match (provider, scope) {
                (CloudProvider::Aws, Some(CloudScope::Aws { profile, .. })) => {
                    format!("aws sso login --profile {profile}")
                }
                (CloudProvider::Aws, _) => "aws sso login --profile <profile>".to_owned(),
                (CloudProvider::Gcp, _) => "gcloud auth login".to_owned(),
                (CloudProvider::Azure, _) => "az login".to_owned(),
            };
            (
                format!(
                    "You are not logged in to {provider}. Run the command below in a terminal, \
                     then refresh."
                ),
                Some(command),
            )
        }
        Issue::CliMissing => (
            format!(
                "The `{cli}` CLI was not found. Install it, or set its path in settings, then \
                 refresh."
            ),
            None,
        ),
        Issue::CliTooOld => (
            format!("This version of the `{cli}` CLI is too old. Upgrade it, then refresh."),
            None,
        ),
        Issue::PluginMissing => match provider {
            CloudProvider::Gcp => (
                "The GKE auth plugin is missing, so clusters cannot be connected. Install it, \
                 then refresh."
                    .to_owned(),
                Some("gcloud components install gke-gcloud-auth-plugin".to_owned()),
            ),
            CloudProvider::Azure => (
                "kubelogin is missing, so clusters cannot be connected. Install it, then refresh."
                    .to_owned(),
                Some("az aks install-cli".to_owned()),
            ),
            CloudProvider::Aws => (
                "A tool needed to connect is missing. Install the AWS CLI v2, then refresh."
                    .to_owned(),
                None,
            ),
        },
        Issue::Forbidden => (
            format!(
                "Your {provider} account is not allowed to list clusters here. Check its role, \
                 or choose another profile, project or subscription."
            ),
            None,
        ),
        Issue::Network => (
            format!("{provider} could not be reached. Check the network connection, then refresh."),
            None,
        ),
        Issue::Timeout => (
            format!("The `{cli}` CLI took too long to answer. Refresh to try again."),
            None,
        ),
        Issue::NoRegions => (
            "No AWS regions are configured to scan. Add regions in the cloud settings, or set a \
             default region for the profile, then refresh."
                .to_owned(),
            Some("aws configure set region <region> --profile <profile>".to_owned()),
        ),
        Issue::Other => (
            "Discovery failed. Refresh to try again; the log has the details.".to_owned(),
            None,
        ),
    };
    Remediation {
        issue,
        text,
        command,
    }
}
