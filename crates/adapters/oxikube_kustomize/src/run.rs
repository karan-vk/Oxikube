//! Running one child process to completion.
//!
//! The child is spawned with `kill_on_drop(true)`: dropping the call's future (cancellation) or
//! passing the deadline kills it. Stdin is closed, stdout and stderr are piped and read
//! concurrently by `wait_with_output`, so a large manifest stream cannot fill a pipe and stall
//! the child. Neither stream is ever logged: stdout may hold `secretGenerator` values.
//! Modelled on `oxikube_helm::cli::run` and `oxikube_cloud::process::runner`.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use oxikube_domain::kustomize::KustomizeError;
use tokio::process::{Child, Command};

/// What a finished process printed, undecoded errors replaced rather than failing.
pub(crate) struct Finished {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// How often [`spawn`] retries a binary that is still open for writing.
const BUSY_RETRIES: u32 = 5;

/// Spawns `command`, retrying while the binary is "text file busy" (`ETXTBSY`): Linux refuses
/// to exec a file some process holds open for writing, which clears within milliseconds.
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

/// Runs `binary args` with `PATH=path_var`, killing it after `timeout`.
pub(crate) async fn run(
    binary: &Path,
    args: &[OsString],
    path_var: &std::ffi::OsStr,
    timeout: Duration,
) -> Result<Finished, KustomizeError> {
    let mut command = Command::new(binary);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PATH", path_var)
        .env("NO_COLOR", "1")
        .kill_on_drop(true);
    let child = spawn(&mut command).await.map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            KustomizeError::NotInstalled
        } else {
            io_error(&error)
        }
    })?;
    // On timeout the future, and with it the child, is dropped: `kill_on_drop` kills it.
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| KustomizeError::Timeout(format!("{}s", timeout.as_secs())))?
        .map_err(|error| io_error(&error))?;
    Ok(Finished {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn io_error(error: &io::Error) -> KustomizeError {
    KustomizeError::Io(oxikube_domain::redact::redact(&error.to_string()).into_owned())
}
