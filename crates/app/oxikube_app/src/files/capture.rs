//! Running one short command in a container and keeping what it printed.

use futures::StreamExt as _;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ExecOptions, ExecStreamPort, ExitStatus};

/// The most stdout kept: a directory of a million entries is an error, not a gigabyte in memory.
pub(super) const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// The most stderr kept (the start of it names the first problem).
const MAX_STDERR_BYTES: usize = 64 * 1024;

/// What a command printed and how it ended.
#[derive(Debug)]
pub(super) struct Captured {
    /// Everything on stdout.
    pub(super) stdout: Vec<u8>,
    /// The start of stderr.
    pub(super) stderr: Vec<u8>,
    /// The exit status.
    pub(super) status: ExitStatus,
}

/// Runs `command` (argv, no TTY, no stdin) in `container` of `namespace/pod` and collects its
/// output, bounded in size. There is no timer here (it would tie the code to one async runtime):
/// the caller runs it as an abort-on-drop task, so leaving the directory ends a listing that hangs
/// and drops the session, which closes the connection. Nothing is logged.
///
/// # Errors
///
/// The open's own error; `Conflict` for more than [`MAX_OUTPUT_BYTES`] of output.
pub(super) async fn capture(
    port: &dyn ExecStreamPort,
    namespace: &str,
    pod: &str,
    container: Option<&str>,
    command: &[String],
) -> OxiResult<Captured> {
    let options = ExecOptions {
        container: container.map(str::to_owned),
        stdin: false,
        stdout: true,
        stderr: true,
        tty: false,
    };
    async {
        let mut session = port.exec_session(namespace, pod, command, &options).await?;
        let mut stdout = session.stdout.take();
        let mut stderr = session.stderr.take();
        let out = async {
            let mut kept = Vec::new();
            if let Some(stream) = stdout.as_mut() {
                while let Some(chunk) = stream.next().await {
                    kept.extend_from_slice(&chunk?);
                    if kept.len() > MAX_OUTPUT_BYTES {
                        return Err(OxiError::conflict(
                            "the directory listing is too large to show",
                        ));
                    }
                }
            }
            Ok::<_, OxiError>(kept)
        };
        let err = async {
            let mut kept = Vec::new();
            // Read to the end even past the cap: an unread pipe would stall the program.
            if let Some(stream) = stderr.as_mut() {
                while let Some(Ok(chunk)) = stream.next().await {
                    let room = MAX_STDERR_BYTES.saturating_sub(kept.len());
                    kept.extend_from_slice(&chunk[..chunk.len().min(room)]);
                }
            }
            kept
        };
        let (stdout, stderr) = futures::join!(out, err);
        let stdout = stdout?;
        let status = session.status.await?;
        Ok(Captured {
            stdout,
            stderr,
            status,
        })
    }
    .await
}
