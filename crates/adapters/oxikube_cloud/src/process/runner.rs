//! [`CliRunner`]: the real [`ProcessRunner`], spawning the CLIs with Tokio.
//!
//! Modelled on the `helm` runner (`oxikube_helm::cli::run`, E16), whose design it copies
//! rather than depends on (adapters never depend on each other): `tokio::process`, stdin
//! closed, stdout and stderr piped, `kill_on_drop(true)` so dropping the call kills the child,
//! a deadline that kills it too, a missing binary as `Unsupported` naming its settings key, and
//! stderr redacted and classified. Added here: a cancellation token, JSON output, a cap on
//! concurrent processes, and the login shell's `PATH`.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxikube_domain::redact::redact;
use tokio::process::Command;
use tokio::sync::{OnceCell, Semaphore};

use super::ProcessRunner;
use super::cli::{Cli, CliInvocation, CliOutput};
use super::error::{CliError, redacted_tail};
use super::path::{LoginShell, SearchPath, is_executable, login_shell_path};

/// How long a command may run when its [`RunOpts`](super::RunOpts) set no timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// How many CLI processes run at once by default.
pub const DEFAULT_MAX_CONCURRENT: usize = 4;

/// How long the login shell may take to report its `PATH`.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// What the user configures for the runner (the `cloud` settings, E18-S07).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRunnerConfig {
    /// Per-program path overrides (`cloud.aws_path`, ...). An override that is not an
    /// executable file makes the program "not found"; it is never searched for elsewhere.
    pub paths: BTreeMap<Cli, PathBuf>,
    /// Timeout of a command whose options set none.
    pub default_timeout: Duration,
    /// Processes allowed to run at once; further runs wait for a slot (at least 1).
    pub max_concurrent: usize,
    /// Which shell to ask for the user's `PATH`.
    pub login_shell: LoginShell,
    /// How long that shell may take.
    pub probe_timeout: Duration,
}

impl Default for CliRunnerConfig {
    /// No overrides, 30 s per command, 4 at once, `$SHELL` probed for up to 5 s.
    fn default() -> Self {
        Self {
            paths: BTreeMap::new(),
            default_timeout: DEFAULT_TIMEOUT,
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            login_shell: LoginShell::FromEnv,
            probe_timeout: DEFAULT_PROBE_TIMEOUT,
        }
    }
}

/// Runs the cloud CLIs. Cheap to share (`Arc<CliRunner>`); every provider adapter uses one.
///
/// Needs a Tokio runtime with the process and time drivers (`oxikube_runtime::spawn_kube`
/// provides it); never call it on the UI thread. The login shell is probed on the first call
/// that needs a binary and the answer is kept for the runner's life.
#[derive(Debug)]
pub struct CliRunner {
    config: CliRunnerConfig,
    slots: Arc<Semaphore>,
    search_path: OnceCell<SearchPath>,
}

impl CliRunner {
    /// A runner with `config`.
    pub fn new(config: CliRunnerConfig) -> Self {
        let slots = Arc::new(Semaphore::new(config.max_concurrent.max(1)));
        Self {
            config,
            slots,
            search_path: OnceCell::new(),
        }
    }

    /// The folders searched, resolving the login shell's `PATH` on first use.
    async fn search_path(&self) -> &SearchPath {
        self.search_path
            .get_or_init(|| async {
                let login = match self.config.login_shell.program() {
                    Some(shell) => login_shell_path(&shell, self.config.probe_timeout).await,
                    None => None,
                };
                if login.is_none() {
                    tracing::debug!("cloud: login shell PATH unavailable; using the app's PATH");
                }
                let process = std::env::var_os("PATH");
                SearchPath::build(login.as_deref(), process.as_deref())
            })
            .await
    }

    async fn run_inner(&self, invocation: &CliInvocation) -> Result<CliOutput, CliError> {
        let cli = invocation.cli;
        let opts = &invocation.opts;
        let cancelled = || CliError::Cancelled {
            cli,
            command: invocation.label(),
        };
        let _slot = tokio::select! {
            biased;
            () = opts.cancel.cancelled() => return Err(cancelled()),
            slot = self.slots.acquire() => slot.map_err(|_| cancelled())?,
        };
        let binary = self.locate(cli).await.ok_or(CliError::NotFound { cli })?;
        let search_path = self.search_path().await;
        let mut command = Command::new(&binary);
        command
            .args(invocation.full_args())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PATH", search_path.as_path_var())
            .env("NO_COLOR", "1")
            .envs(cli.quiet_env().iter().copied())
            .envs(opts.env.iter().map(|(name, value)| (name, value)))
            .kill_on_drop(true);
        let child = command
            .spawn()
            .map_err(|error| spawn_error(invocation, &error))?;
        let after = opts.timeout.unwrap_or(self.config.default_timeout);
        // Leaving the select drops `wait_with_output`, and with it the child, on cancel or
        // timeout: `kill_on_drop` kills the process.
        let output = tokio::select! {
            biased;
            () = opts.cancel.cancelled() => return Err(cancelled()),
            output = tokio::time::timeout(after, child.wait_with_output()) => output,
        };
        let output = output
            .map_err(|_| CliError::Timeout {
                cli,
                command: invocation.label(),
                after,
            })?
            .map_err(|error| io_error(invocation, &error))?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.success() {
            return Ok(CliOutput {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: redacted_tail(&stderr),
            });
        }
        Err(CliError::exit(
            cli,
            invocation.label(),
            output.status.code(),
            &stderr,
        ))
    }
}

impl Default for CliRunner {
    fn default() -> Self {
        Self::new(CliRunnerConfig::default())
    }
}

fn spawn_error(invocation: &CliInvocation, error: &io::Error) -> CliError {
    if error.kind() == io::ErrorKind::NotFound {
        return CliError::NotFound {
            cli: invocation.cli,
        };
    }
    io_error(invocation, error)
}

fn io_error(invocation: &CliInvocation, error: &io::Error) -> CliError {
    CliError::Io {
        cli: invocation.cli,
        command: invocation.label(),
        reason: redact(&error.to_string()).into_owned(),
    }
}

#[async_trait]
impl ProcessRunner for CliRunner {
    async fn locate(&self, cli: Cli) -> Option<PathBuf> {
        if let Some(path) = self.config.paths.get(&cli) {
            return is_executable(path).then(|| path.clone());
        }
        self.search_path().await.find(cli)
    }

    async fn run(&self, invocation: CliInvocation) -> Result<CliOutput, CliError> {
        let result = self.run_inner(&invocation).await;
        // Only the command's label and its outcome: arguments, output and stderr stay out of
        // the log (non-negotiable 5).
        let command = invocation.label();
        match &result {
            Ok(_) => tracing::debug!(%command, "cloud cli succeeded"),
            Err(CliError::Exit { code, .. }) => {
                tracing::debug!(%command, ?code, "cloud cli failed");
            }
            Err(error) => tracing::debug!(%command, kind = error_kind(error), "cloud cli failed"),
        }
        result
    }
}

fn error_kind(error: &CliError) -> &'static str {
    match error {
        CliError::NotFound { .. } => "not_found",
        CliError::Exit { .. } => "exit",
        CliError::Timeout { .. } => "timeout",
        CliError::Cancelled { .. } => "cancelled",
        CliError::InvalidJson { .. } => "invalid_json",
        CliError::Io { .. } => "io",
    }
}
