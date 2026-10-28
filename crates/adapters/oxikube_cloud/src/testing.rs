//! [`RecordedRunner`]: a [`ProcessRunner`] that answers from recorded CLI output, so the
//! provider adapters' parsers and error handling are tested without a CLI, an account or a
//! network (feature `testing`, and this crate's own tests).
//!
//! Recordings live next to the adapters under `tests/fixtures/`, named by CLI version
//! (`aws-2.15/eks-list-clusters.json`), and are loaded with [`Reply::stdout_file`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::process::{Cli, CliError, CliInvocation, CliOutput, OutputFormat, ProcessRunner};

/// What a recorded command answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// Exit 0 printing this on stdout.
    Stdout(String),
    /// Exit `code` printing `stderr` (redacted on the way out, as the real runner does).
    Fail {
        /// Exit code.
        code: i32,
        /// What the CLI printed on stderr.
        stderr: String,
    },
    /// The command ran past its timeout.
    Timeout,
}

impl Reply {
    /// Exit 0 printing the content of `path` (a recorded fixture).
    ///
    /// # Panics
    ///
    /// When the file cannot be read: a missing fixture is a broken test.
    pub fn stdout_file(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("fixture {} unreadable: {error}", path.display()));
        Self::Stdout(text)
    }

    /// Exit `code` printing `stderr`.
    pub fn fail(code: i32, stderr: impl Into<String>) -> Self {
        Self::Fail {
            code,
            stderr: stderr.into(),
        }
    }
}

/// One call a [`RecordedRunner`] answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCall {
    /// The program.
    pub cli: Cli,
    /// Its arguments, without the JSON flags.
    pub args: Vec<String>,
    /// Text or JSON.
    pub format: OutputFormat,
    /// The extra environment it was given.
    pub env: Vec<(String, String)>,
}

type Key = (Cli, Vec<String>);

/// A [`ProcessRunner`] answering each `(cli, args)` with a recorded [`Reply`]. Programs not
/// marked [`installed`](Self::installed) are not found; a command with no recording fails
/// with an `Io` error naming it. Replies are reused, so one recording serves repeated calls.
#[derive(Debug, Default)]
pub struct RecordedRunner {
    installed: BTreeMap<Cli, PathBuf>,
    replies: BTreeMap<Key, Reply>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl RecordedRunner {
    /// A runner where nothing is installed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks `cli` installed at `/usr/local/bin/<name>`.
    #[must_use]
    pub fn installed(mut self, cli: Cli) -> Self {
        let path = PathBuf::from("/usr/local/bin").join(cli.name());
        self.installed.insert(cli, path);
        self
    }

    /// Answers `cli args` (the arguments without the JSON flags) with `reply`.
    #[must_use]
    pub fn on(mut self, cli: Cli, args: &[&str], reply: Reply) -> Self {
        let args = args.iter().map(|arg| (*arg).to_owned()).collect();
        self.replies.insert((cli, args), reply);
        self
    }

    /// Every call answered so far, in order.
    pub fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().clone()
    }
}

#[async_trait]
impl ProcessRunner for RecordedRunner {
    async fn locate(&self, cli: Cli) -> Option<PathBuf> {
        self.installed.get(&cli).cloned()
    }

    async fn run(&self, invocation: CliInvocation) -> Result<CliOutput, CliError> {
        let cli = invocation.cli;
        let command = invocation.label();
        self.calls.lock().push(RecordedCall {
            cli,
            args: invocation.args.clone(),
            format: invocation.format,
            env: invocation.opts.env.clone(),
        });
        if invocation.opts.cancel.is_cancelled() {
            return Err(CliError::Cancelled { cli, command });
        }
        if !self.installed.contains_key(&cli) {
            return Err(CliError::NotFound { cli });
        }
        match self.replies.get(&(cli, invocation.args)) {
            Some(Reply::Stdout(stdout)) => Ok(CliOutput {
                stdout: stdout.clone(),
                stderr: String::new(),
            }),
            Some(Reply::Fail { code, stderr }) => {
                Err(CliError::exit(cli, command, Some(*code), stderr))
            }
            Some(Reply::Timeout) => Err(CliError::Timeout {
                cli,
                command,
                after: invocation.opts.timeout.unwrap_or(Duration::from_secs(30)),
            }),
            None => Err(CliError::Io {
                cli,
                command: command.clone(),
                reason: format!("no recording for `{command}`"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use oxikube_domain::{ErrorKind, OxiError};

    use super::*;

    #[tokio::test]
    async fn replies_by_command_and_parses_json() {
        let runner = RecordedRunner::new()
            .installed(Cli::Aws)
            .on(
                Cli::Aws,
                &["eks", "list-clusters"],
                Reply::Stdout(r#"{"clusters":["prod","dev"]}"#.into()),
            )
            .on(
                Cli::Aws,
                &["eks", "describe-cluster"],
                Reply::fail(255, "Unable to locate credentials"),
            );
        let runner: Arc<dyn ProcessRunner> = Arc::new(runner);
        let listed: serde_json::Value = runner
            .run_json(CliInvocation::new(Cli::Aws, ["eks", "list-clusters"]))
            .await
            .unwrap();
        assert_eq!(listed["clusters"][1], "dev");
        let error = runner
            .run(CliInvocation::new(Cli::Aws, ["eks", "describe-cluster"]))
            .await
            .unwrap_err();
        assert!(error.needs_login());
        assert_eq!(OxiError::from(error).kind(), ErrorKind::Auth);
        let missing = runner.run(CliInvocation::new(Cli::Gcloud, ["info"])).await;
        assert_eq!(
            missing.unwrap_err(),
            CliError::NotFound { cli: Cli::Gcloud }
        );
        assert_eq!(
            runner.locate(Cli::Aws).await,
            Some("/usr/local/bin/aws".into())
        );
    }

    #[tokio::test]
    async fn unrecorded_commands_and_bad_json_fail_clearly() {
        let runner = RecordedRunner::new().installed(Cli::Az).on(
            Cli::Az,
            &["aks", "list"],
            Reply::Stdout("not json".into()),
        );
        let shared: Arc<dyn ProcessRunner> = Arc::new(runner);
        let bad = shared
            .run_json::<serde_json::Value>(CliInvocation::new(Cli::Az, ["aks", "list"]))
            .await
            .unwrap_err();
        assert!(matches!(bad, CliError::InvalidJson { .. }), "{bad:?}");
        let unrecorded = shared
            .run(CliInvocation::new(Cli::Az, ["account", "list"]))
            .await;
        assert!(unrecorded.unwrap_err().to_string().contains("no recording"));
    }
}
