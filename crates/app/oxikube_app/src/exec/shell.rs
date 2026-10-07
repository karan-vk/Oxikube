//! Finding the shell a container has: a quick exec of each shell of the chain.
//!
//! The probe runs `<shell> -c "exit 0"` without a TTY or stdin and reads the exit: success means
//! the shell is there, exit 126/127 (or the runtime's `executable file not found`) means it is
//! not, and anything else is a real failure (no permission, pod gone). It asks the container
//! instead of guessing from the image, and costs one more round trip before the shell opens.

use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{BackendEvent, ExecPort, ExecTarget};

use super::failure::{error_is_missing_program, is_missing_program};

/// What the probe found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Probe {
    /// The shell exists in the container.
    Present,
    /// The container has no such program.
    Missing,
}

/// Probes `shell` in `container` of `pod` (`None`: the API server's default).
///
/// # Errors
///
/// Whatever stops the container from answering (`Forbidden`, `NotFound`, `Conflict` for a
/// container that does not run, `Timeout` after `timeout`); a missing shell is
/// [`Probe::Missing`], not an error.
pub(super) async fn probe_shell(
    port: &dyn ExecPort,
    pod: &ResourceRef,
    container: Option<&str>,
    shell: &str,
    timeout: Duration,
) -> OxiResult<Probe> {
    let target = ExecTarget {
        pod: pod.clone(),
        container: container.map(str::to_owned),
        command: vec![shell.to_owned(), "-c".to_owned(), "exit 0".to_owned()],
        tty: false,
        stdin: false,
    };
    let run = async {
        let backend = match port.exec(&target).await {
            Ok(backend) => backend,
            Err(error) if error_is_missing_program(&error) => return Ok(Probe::Missing),
            Err(error) => return Err(error),
        };
        let mut events = backend.output_stream();
        // Output of the probe (a shell's own complaint) is dropped unread: it can say anything.
        while let Some(event) = events.next().await {
            match event {
                BackendEvent::Output(_) => {}
                BackendEvent::Exited(status) if status.is_success() => return Ok(Probe::Present),
                BackendEvent::Exited(status) if is_missing_program(&status) => {
                    return Ok(Probe::Missing);
                }
                // No exit code and no signal: the command never ran (the runtime refused it).
                BackendEvent::Exited(status)
                    if status.code.is_none() && status.signal.is_none() =>
                {
                    return Err(OxiError::conflict(status.message.unwrap_or_else(|| {
                        "the container could not run the shell probe".to_owned()
                    })));
                }
                // The shell ran and exited non-zero for its own reasons: it exists.
                BackendEvent::Exited(_) => return Ok(Probe::Present),
                BackendEvent::Error(error) if error_is_missing_program(&error) => {
                    return Ok(Probe::Missing);
                }
                BackendEvent::Error(error) => return Err(error),
            }
        }
        Err(OxiError::network(
            "the connection closed while looking for a shell",
        ))
    };
    tokio::time::timeout(timeout, run)
        .await
        .unwrap_or_else(|_| {
            Err(OxiError::timeout(
                "looking for a shell in the container timed out",
            ))
        })
}
