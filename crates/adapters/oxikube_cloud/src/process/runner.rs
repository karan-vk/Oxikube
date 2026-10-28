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
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use tokio::process::{Child, Command};

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
    /// Swapped by [`reconfigure`](Self::reconfigure); every run reads it once at its start.
    config: RwLock<CliRunnerConfig>,
    slots: RwLock<Arc<Semaphore>>,
    search_path: OnceCell<SearchPath>,
}

impl CliRunner {
    /// A runner with `config`.
    pub fn new(config: CliRunnerConfig) -> Self {
        let slots = Arc::new(Semaphore::new(config.max_concurrent.max(1)));
        Self {
            config: RwLock::new(config),
            slots: RwLock::new(slots),
            search_path: OnceCell::new(),
        }
    }

    /// Applies a changed configuration (the `cloud` settings, hot reload): path overrides and
    /// the default timeout take effect for the next command; a new `max_concurrent` gives later
    /// commands a new slot pool (commands already running keep their slot, so for a moment both
    /// pools may be in use). The login shell and its probe timeout are read once and keep their
    /// first values.
    pub fn reconfigure(&self, config: CliRunnerConfig) {
        if config.max_concurrent.max(1) != self.snapshot().max_concurrent.max(1) {
            *write(&self.slots) = Arc::new(Semaphore::new(config.max_concurrent.max(1)));
        }
        *write(&self.config) = config;
    }

    /// The configuration now in force.
    fn snapshot(&self) -> CliRunnerConfig {
        read(&self.config).clone()
    }

    /// The folders searched, resolving the login shell's `PATH` on first use.
    async fn search_path(&self) -> &SearchPath {
        self.search_path
            .get_or_init(|| async {
                let config = self.snapshot();
                let login = match config.login_shell.program() {
                    Some(shell) => login_shell_path(&shell, config.probe_timeout).await,
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
        let slots = read(&self.slots).clone();
        let _slot = tokio::select! {
            biased;
            () = opts.cancel.cancelled() => return Err(cancelled()),
            slot = slots.acquire() => slot.map_err(|_| cancelled())?,
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
        let child = spawn(&mut command)
            .await
            .map_err(|error| spawn_error(invocation, &error))?;
        let after = opts
            .timeout
            .unwrap_or_else(|| self.snapshot().default_timeout);
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

fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

impl Default for CliRunner {
    fn default() -> Self {
        Self::new(CliRunnerConfig::default())
    }
}

/// How often [`spawn`] retries a binary that is still open for writing.
const BUSY_RETRIES: u32 = 5;

/// Spawns `command`, retrying while the binary is "text file busy" (`ETXTBSY`).
///
/// Linux refuses to exec a file that some process holds open for writing. That happens while a
/// CLI is being installed or upgraded, and when another thread forks between writing a script
/// and closing it: the child holds the write descriptor until it execs. Both clear within
/// milliseconds, so a few short retries turn a spurious failure into a run.
async fn spawn(command: &mut Command) -> io::Result<Child> {
    let mut attempt = 0;
    loop {
        match command.spawn() {
            Err(error)
                if error.kind() == io::ErrorKind::ExecutableFileBusy && attempt < BUSY_RETRIES =>
            {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(5 * u64::from(attempt))).await;
            }
            result => return result,
        }
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
        let overridden = self.snapshot().paths.get(&cli).cloned();
        if let Some(path) = overridden {
            return is_executable(&path).then_some(path);
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
