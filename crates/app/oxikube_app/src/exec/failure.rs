//! Telling a missing shell from a failure, and turning a failed open into a message the
//! terminal tab can show.

use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::ExitStatus;

/// Whether a command that ended with `status` never ran because its program is not in the
/// container. The rule is shared with the tar transfer: [`ExitStatus::is_missing_program`].
pub(super) fn is_missing_program(status: &ExitStatus) -> bool {
    status.is_missing_program()
}

/// Whether a failed open says the program is not in the container.
pub(super) fn error_is_missing_program(error: &OxiError) -> bool {
    oxikube_ports::exec::error_is_missing_program(error)
}

/// A failed open, as a message the user can act on. The adapter's messages already name the pod
/// and the permission that is missing; this adds what to do next for the failures that go away
/// by themselves, and marks them retryable.
pub(super) fn explain_open(error: OxiError) -> OxiError {
    match error.kind() {
        ErrorKind::Conflict => OxiError::conflict(format!(
            "{}. Open it again once the container is running.",
            error.message().trim_end_matches('.')
        ))
        .with_retryable(true),
        _ => error,
    }
}

/// Why no shell of the chain opened: none of `tried` is in the container.
pub(super) fn no_shell(tried: &[String], container: Option<&str>, windows: bool) -> OxiError {
    let list = tried.join(", ");
    let place = container.map_or_else(
        || "the container".to_owned(),
        |name| format!("container {name}"),
    );
    let advice = if windows {
        "The pod runs Windows containers, which have neither: set `terminal.exec_shells` to \
         [\"powershell\", \"cmd\"], or open a debug container instead."
    } else {
        "It may be a distroless or scratch image: open a debug container instead (it brings its \
         own shell), or list a shell it has in `terminal.exec_shells`."
    };
    OxiError::unsupported(format!("No shell ({list}) was found in {place}. {advice}"))
}
