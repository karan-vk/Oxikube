//! [`CliError`]: why a CLI run failed, and its mapping onto the port error taxonomy.
//!
//! Stderr is redacted ([`oxikube_domain::redact`]) before it is stored in an error, so neither
//! the error nor a log line built from it can carry a token the CLI echoed.

use std::time::Duration;

use oxikube_domain::OxiError;
use oxikube_domain::redact::redact;

use super::cli::Cli;
use super::path::EXTRA_DIRS;

/// Longest stderr tail kept in an error, in characters (the end is where CLIs report).
const STDERR_MAX: usize = 4000;

/// Longest stderr summary put in a user-facing message, in characters.
const SUMMARY_MAX: usize = 600;

/// Why a CLI run failed. Every text field is redacted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CliError {
    /// No executable at the configured path, on the `PATH` or in the usual folders.
    #[error("`{cli}` was not found")]
    NotFound {
        /// The program.
        cli: Cli,
    },
    /// The process ran and exited unsuccessfully.
    #[error("`{command}` failed (exit {code:?}): {stderr}")]
    Exit {
        /// The program.
        cli: Cli,
        /// The command's [label](super::CliInvocation::label).
        command: String,
        /// Exit code, `None` when killed by a signal.
        code: Option<i32>,
        /// The end of stderr, redacted.
        stderr: String,
    },
    /// It ran past its timeout and was killed.
    #[error("`{command}` did not finish within {after:?}")]
    Timeout {
        /// The program.
        cli: Cli,
        /// The command's label.
        command: String,
        /// The timeout.
        after: Duration,
    },
    /// The caller cancelled it; the process was killed (or never started).
    #[error("`{command}` was cancelled")]
    Cancelled {
        /// The program.
        cli: Cli,
        /// The command's label.
        command: String,
    },
    /// It succeeded but stdout was not the JSON expected.
    #[error("`{command}` printed output that is not the expected JSON: {reason}")]
    InvalidJson {
        /// The program.
        cli: Cli,
        /// The command's label.
        command: String,
        /// The parser's complaint, redacted.
        reason: String,
    },
    /// It could not be started or its pipes failed.
    #[error("`{command}` could not be run: {reason}")]
    Io {
        /// The program.
        cli: Cli,
        /// The command's label.
        command: String,
        /// The OS error, redacted.
        reason: String,
    },
}

impl CliError {
    /// A failed exit, keeping the redacted end of `stderr`.
    pub fn exit(cli: Cli, command: String, code: Option<i32>, stderr: &str) -> Self {
        Self::Exit {
            cli,
            command,
            code,
            stderr: redacted_tail(stderr),
        }
    }

    /// The program that failed.
    #[must_use]
    pub fn cli(&self) -> Cli {
        match self {
            Self::NotFound { cli }
            | Self::Exit { cli, .. }
            | Self::Timeout { cli, .. }
            | Self::Cancelled { cli, .. }
            | Self::InvalidJson { cli, .. }
            | Self::Io { cli, .. } => *cli,
        }
    }

    /// The redacted stderr of a failed exit.
    #[must_use]
    pub fn stderr(&self) -> Option<&str> {
        match self {
            Self::Exit { stderr, .. } => Some(stderr),
            Self::NotFound { .. }
            | Self::Timeout { .. }
            | Self::Cancelled { .. }
            | Self::InvalidJson { .. }
            | Self::Io { .. } => None,
        }
    }

    /// `true` when a failed exit's stderr contains `needle` (ASCII case-insensitive), for the
    /// provider-specific checks of the adapters.
    #[must_use]
    pub fn says(&self, needle: &str) -> bool {
        let needle = needle.to_ascii_lowercase();
        self.stderr()
            .is_some_and(|stderr| stderr.to_ascii_lowercase().contains(&needle))
    }

    /// `true` when the CLI said its login is missing or expired.
    #[must_use]
    pub fn needs_login(&self) -> bool {
        self.stderr()
            .is_some_and(|stderr| needs_login(&stderr.to_ascii_lowercase()))
    }
}

/// The redacted, capped end of `text`.
pub(crate) fn redacted_tail(text: &str) -> String {
    let redacted = redact(text.trim());
    let count = redacted.chars().count();
    match redacted
        .char_indices()
        .nth(count.saturating_sub(STDERR_MAX))
    {
        Some((start, _)) if start > 0 => format!("…{}", &redacted[start..]),
        Some(_) | None => redacted.into_owned(),
    }
}

/// A short summary of (already redacted) `stderr`: its error lines (`ERROR: ...`,
/// `An error occurred ...`), else its last lines; warnings dropped; capped.
pub(crate) fn summary(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let is_error = |line: &&str| {
        let lower = line.to_ascii_lowercase();
        lower.starts_with("error") || lower.starts_with("an error occurred")
    };
    let errors: Vec<&str> = lines.iter().copied().filter(is_error).collect();
    let picked = if errors.is_empty() {
        let kept: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| !l.to_ascii_lowercase().starts_with("warning"))
            .collect();
        let tail = if kept.is_empty() {
            &lines[..]
        } else {
            &kept[..]
        };
        tail[tail.len().saturating_sub(3)..].join("; ")
    } else {
        errors.join("; ")
    };
    match picked.char_indices().nth(SUMMARY_MAX) {
        Some((cut, _)) => format!("{}…", &picked[..cut]),
        None => picked,
    }
}

/// Phrases (lower case) that mean the login is missing or expired, from the recorded stderr
/// of `aws` v2, `gcloud`, `az`, `kubelogin` and `gke-gcloud-auth-plugin`.
const LOGIN_PHRASES: &[&str] = &[
    "unable to locate credentials",
    "token has expired",
    "sso session",
    "expiredtoken",
    "security token included in the request is expired",
    "you do not currently have an active account selected",
    "reauthentication",
    "gcloud auth login",
    "refresh token has expired",
    "az login",
    "please run 'az login'",
    "aadsts",
    "interactive authentication is needed",
    "log in again",
];

const FORBIDDEN_PHRASES: &[&str] = &[
    "accessdenied",
    "access denied",
    "not authorized to perform",
    "permission_denied",
    "permission denied",
    "authorizationfailed",
    "does not have authorization",
];

const NETWORK_PHRASES: &[&str] = &[
    "could not connect to the endpoint url",
    "connection refused",
    "name or service not known",
    "nodename nor servname",
    "temporary failure in name resolution",
    "network is unreachable",
    "failed to establish a new connection",
    "connection reset",
    "read timed out",
];

const USAGE_PHRASES: &[&str] = &[
    "invalid choice",
    "unrecognized arguments",
    "is misspelled or not recognized",
    "unknown flag",
    "unknown command",
];

fn needs_login(lower: &str) -> bool {
    LOGIN_PHRASES.iter().any(|p| lower.contains(p))
}

fn says_any(lower: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| lower.contains(p))
}

/// Maps a failed exit by what the CLI said.
fn classify(cli: Cli, command: &str, code: Option<i32>, stderr: &str) -> OxiError {
    let lower = stderr.to_ascii_lowercase();
    let said = summary(stderr);
    if needs_login(&lower) {
        OxiError::auth(
            format!("`{cli}` is not logged in: {}. ({said})", cli.login_hint()),
            false,
        )
    } else if says_any(&lower, FORBIDDEN_PHRASES) {
        OxiError::forbidden(format!("`{command}` was denied: {said}"))
    } else if says_any(&lower, NETWORK_PHRASES) {
        OxiError::network(format!("`{command}` could not reach the provider: {said}"))
    } else if says_any(&lower, USAGE_PHRASES) {
        OxiError::unsupported(format!(
            "this version of `{cli}` does not support `{command}`; upgrade it ({said})"
        ))
    } else {
        let code = code.map_or_else(|| "a signal".to_owned(), |c| c.to_string());
        OxiError::internal(format!("`{command}` failed (exit {code}): {said}"))
    }
}

impl From<CliError> for OxiError {
    fn from(error: CliError) -> Self {
        match error {
            CliError::NotFound { cli } => OxiError::unsupported(format!(
                "`{cli}` was not found on the login shell's PATH or in {}: install it or set \
                 `{}` in settings",
                EXTRA_DIRS.join(", "),
                cli.setting_key()
            )),
            CliError::Exit {
                cli,
                command,
                code,
                stderr,
            } => classify(cli, &command, code, &stderr),
            CliError::Timeout { command, after, .. } => OxiError::timeout(format!(
                "`{command}` did not finish within {}s and was stopped",
                after.as_secs_f32()
            )),
            CliError::Cancelled { command, .. } => {
                OxiError::internal(format!("`{command}` was cancelled"))
            }
            CliError::InvalidJson {
                command, reason, ..
            } => OxiError::internal(format!(
                "`{command}` printed output Oxikube does not recognise (a newer CLI?): {reason}"
            )),
            CliError::Io {
                command, reason, ..
            } => OxiError::internal(format!("`{command}` could not be run: {reason}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ErrorKind;

    use super::*;

    fn exit(cli: Cli, stderr: &str) -> OxiError {
        CliError::exit(cli, format!("{cli} x"), Some(1), stderr).into()
    }

    #[test]
    fn login_failures_are_auth_with_the_cli_hint() {
        let cases = [
            (
                Cli::Aws,
                "Unable to locate credentials. You can configure credentials by running \"aws configure\".",
            ),
            (
                Cli::Aws,
                "Error when retrieving token from sso: Token has expired and refresh failed",
            ),
            (
                Cli::Gcloud,
                "ERROR: (gcloud.container.clusters.list) You do not currently have an active account selected.",
            ),
            (Cli::Az, "ERROR: Please run 'az login' to setup account."),
            (
                Cli::Kubelogin,
                "AADSTS700082: The refresh token has expired due to inactivity.",
            ),
        ];
        for (cli, stderr) in cases {
            let error = exit(cli, stderr);
            assert_eq!(error.kind(), ErrorKind::Auth, "{stderr}");
            assert!(
                error.message().contains(cli.login_hint()),
                "{}",
                error.message()
            );
            assert!(CliError::exit(cli, String::new(), Some(1), stderr).needs_login());
        }
    }

    #[test]
    fn denials_network_and_usage_errors_are_classified() {
        let denied = "An error occurred (AccessDeniedException) when calling the ListClusters operation: User: arn:aws:iam::000000000000:user/x is not authorized to perform: eks:ListClusters";
        assert_eq!(exit(Cli::Aws, denied).kind(), ErrorKind::Forbidden);
        let offline = "Could not connect to the endpoint URL: \"https://eks.eu-west-1.amazonaws.com/clusters\"";
        let error = exit(Cli::Aws, offline);
        assert_eq!(error.kind(), ErrorKind::Network);
        assert!(error.is_retryable());
        let usage = "aws: error: argument operation: Invalid choice, valid choices are:";
        assert_eq!(exit(Cli::Aws, usage).kind(), ErrorKind::Unsupported);
        assert_eq!(
            exit(Cli::Az, "ERROR: something odd").kind(),
            ErrorKind::Internal
        );
    }

    #[test]
    fn stderr_is_redacted_before_it_is_kept() {
        let jwt = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjEifQ.eyJzdWIiOiJ4In0.c2lnbmF0dXJl";
        let error = CliError::exit(
            Cli::Aws,
            "aws eks".into(),
            Some(255),
            &format!("ERROR: bad token {jwt}"),
        );
        assert!(!error.stderr().unwrap().contains(jwt));
        assert!(!OxiError::from(error).message().contains(jwt));
    }

    #[test]
    fn summaries_prefer_error_lines_and_are_capped() {
        let stderr = "WARNING: old python\nsome progress\nERROR: (gcloud) the real reason\n";
        assert_eq!(summary(stderr), "ERROR: (gcloud) the real reason");
        assert_eq!(summary("a\nb\nc\nd\nWARNING: x"), "b; c; d");
        assert!(summary(&"x".repeat(5000)).chars().count() <= SUMMARY_MAX + 1);
        assert!(redacted_tail(&"y".repeat(9000)).starts_with('…'));
    }

    #[test]
    fn not_found_names_the_settings_key() {
        let error = OxiError::from(CliError::NotFound { cli: Cli::Gcloud });
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert!(
            error.message().contains("cloud.gcloud_path"),
            "{}",
            error.message()
        );
        let timeout = OxiError::from(CliError::Timeout {
            cli: Cli::Az,
            command: "az aks list".into(),
            after: Duration::from_secs(30),
        });
        assert_eq!(timeout.kind(), ErrorKind::Timeout);
    }
}
