//! Telling a missing shell from a failure, and turning a failed open into a message the
//! terminal tab can show.

use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::ExitStatus;

/// Exit codes a shell returns when the program is not there (126: found but not executable,
/// 127: not found).
const MISSING_CODES: [i32; 2] = [126, 127];

/// Text a runtime puts in the error when the program does not exist in the container.
const MISSING_MARKERS: [&str; 5] = [
    "executable file not found",
    "not found in $path",
    "no such file or directory",
    "command not found",
    "cannot find the file",
];

fn says_missing(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    MISSING_MARKERS.iter().any(|marker| text.contains(marker))
}

/// Whether a command that ended with `status` never ran because its program is not in the
/// container (exit 126 / 127, or the runtime said `executable file not found`).
pub(super) fn is_missing_program(status: &ExitStatus) -> bool {
    status
        .code
        .is_some_and(|code| MISSING_CODES.contains(&code))
        || status.message.as_deref().is_some_and(says_missing)
}

/// Whether a failed open says the program is not in the container.
pub(super) fn error_is_missing_program(error: &OxiError) -> bool {
    says_missing(error.message())
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
