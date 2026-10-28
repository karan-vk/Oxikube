//! Running the cloud CLIs: [`CliRunner`] behind the [`ProcessRunner`] seam.
//!
//! Every provider adapter runs its CLI through a `ProcessRunner` (`Arc<dyn ProcessRunner>`):
//! the real [`CliRunner`] in the app, a recorded one (`testing::RecordedRunner`) in tests,
//! so parsers and error handling are tested on recorded output without any cloud account.
//!
//! - [`Cli`]: which program (`aws`, `gcloud`, `az`, `gke-gcloud-auth-plugin`, `kubelogin`),
//!   its JSON flags, its settings key and its login hint.
//! - [`CliInvocation`] / [`RunOpts`]: arguments, [`OutputFormat`], timeout, a
//!   `CancellationToken`, extra environment.
//! - [`CliError`]: not found, failed exit (redacted stderr), timeout, cancelled, invalid JSON;
//!   `From<CliError> for OxiError` classifies it (not logged in is `Auth` with the CLI's login
//!   hint, denied is `Forbidden`, offline is `Network`, missing is `Unsupported` naming the
//!   settings key).
//! - [`LoginShell`] / [`login_shell_path`]: the user's real `PATH`.
//!
//! # Security
//!
//! Credentials never go on a command line or into the environment the runner sets, stderr
//! is redacted before it is kept, and the log records only a command's label
//! (`aws eks list-clusters`) and outcome.

mod cli;
mod error;
mod path;
mod runner;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use std::path::PathBuf;

pub use cli::{Cli, CliInvocation, CliOutput, OutputFormat, RunOpts};
pub use error::CliError;
pub use path::{EXTRA_DIRS, LoginShell, login_shell_path};
pub use runner::{
    CliRunner, CliRunnerConfig, DEFAULT_MAX_CONCURRENT, DEFAULT_PROBE_TIMEOUT, DEFAULT_TIMEOUT,
};

/// Runs one CLI command to completion. Implemented by [`CliRunner`] and, for tests, by
/// `testing::RecordedRunner`.
#[async_trait]
pub trait ProcessRunner: Send + Sync {
    /// Where `cli` is: its path override when set (and executable), else the first match on
    /// the login shell's `PATH`, the app's `PATH` and [`EXTRA_DIRS`]. `None` when missing.
    async fn locate(&self, cli: Cli) -> Option<PathBuf>;

    /// Runs `invocation` and returns what it printed, or why it failed. A non-zero exit is
    /// [`CliError::Exit`] with redacted stderr. Dropping the future, cancelling the token or
    /// passing the timeout kills the process.
    async fn run(&self, invocation: CliInvocation) -> Result<CliOutput, CliError>;
}

impl dyn ProcessRunner {
    /// Runs `invocation` with JSON output (the CLI's JSON flags appended) and parses stdout
    /// as `T`. Output that does not parse is [`CliError::InvalidJson`], with the parser's
    /// message redacted.
    pub async fn run_json<T: DeserializeOwned>(
        &self,
        invocation: CliInvocation,
    ) -> Result<T, CliError> {
        let invocation = invocation.json();
        let cli = invocation.cli;
        let command = invocation.label();
        let output = self.run(invocation).await?;
        serde_json::from_str(&output.stdout).map_err(|error| CliError::InvalidJson {
            cli,
            command,
            reason: oxikube_domain::redact::redact(&error.to_string()).into_owned(),
        })
    }
}
