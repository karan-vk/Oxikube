//! `ExecOptions` to kube's `AttachParams`, with the checks the server would otherwise answer
//! with an opaque 400.

use kube::api::AttachParams;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::ExecOptions;

use crate::subresource::segment;

/// Capacity of each in-process pipe between the websocket task and the session's streams, in
/// bytes. A full pipe stops the websocket task reading, so the server's send window fills and
/// a flood of output slows the container down instead of growing memory. Kube's default
/// (1 KiB) would turn a burst into many tiny reads.
pub(super) const STREAM_BUFFER: usize = 32 * 1024;

/// The names of the target and the options that cannot reach the server unchanged.
pub(super) fn check_target(namespace: &str, pod: &str, options: &ExecOptions) -> OxiResult<()> {
    segment("a namespace", namespace)?;
    segment("a pod name", pod)?;
    if let Some(container) = &options.container {
        segment("a container name", container)?;
    }
    if options.stderr && options.tty {
        return Err(OxiError::validation(
            "a TTY merges stderr into stdout: ask for stderr only without a TTY",
        ));
    }
    if !options.is_valid() {
        return Err(OxiError::validation(
            "attach at least one of stdin, stdout or stderr",
        ));
    }
    Ok(())
}

/// The command to run: not empty, and its program not blank.
pub(super) fn check_command(command: &[String]) -> OxiResult<()> {
    match command.first() {
        Some(program) if !program.trim().is_empty() => Ok(()),
        _ => Err(OxiError::validation("an exec command cannot be empty")),
    }
}

/// The kube parameters for `options`.
pub(super) fn attach_params(options: &ExecOptions) -> AttachParams {
    let mut params = AttachParams::default()
        .stdin(options.stdin)
        .stdout(options.stdout)
        .stderr(options.stderr)
        .tty(options.tty)
        .max_stdin_buf_size(STREAM_BUFFER)
        .max_stdout_buf_size(STREAM_BUFFER)
        .max_stderr_buf_size(STREAM_BUFFER);
    params.container = options.container.clone();
    params
}
