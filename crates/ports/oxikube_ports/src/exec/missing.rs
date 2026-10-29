//! Telling "the program is not in the container" from any other failure of an exec.
//!
//! One rule for every caller that runs a program it is not sure the image has: the shell search
//! (`oxikube_app::exec`), the tar transfer ([`transfer`](super::transfer)) and its `cat`
//! fallback. A distroless or scratch image has neither a shell nor `tar`.

use oxikube_domain::OxiError;

use super::ExitStatus;

/// Exit codes a runtime or shell returns when the program is not there (126: found but not
/// executable, 127: not found).
const MISSING_CODES: [i32; 2] = [126, 127];

/// Text a runtime puts in its error when the program does not exist in the container.
const MISSING_MARKERS: [&str; 5] = [
    "executable file not found",
    "not found in $path",
    "no such file or directory",
    "command not found",
    "cannot find the file",
];

/// Whether `text` (a runtime's error or status message) says the program to run does not exist.
///
/// Meant for the message of a failed open or of an [`ExitStatus`], which the runtime writes.
/// Never apply it to what the program itself printed: `tar: x: No such file or directory` is a
/// missing *file*, not a missing `tar`.
pub fn says_program_missing(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    MISSING_MARKERS.iter().any(|marker| text.contains(marker))
}

impl ExitStatus {
    /// Whether a command that ended with this status never ran because its program is not in the
    /// container (exit 126 / 127, or the runtime said `executable file not found`).
    pub fn is_missing_program(&self) -> bool {
        self.code.is_some_and(|code| MISSING_CODES.contains(&code))
            || self.message.as_deref().is_some_and(says_program_missing)
    }
}

/// Whether a failed open says the program is not in the container.
pub fn error_is_missing_program(error: &OxiError) -> bool {
    says_program_missing(error.message())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_codes_and_runtime_messages_mean_missing() {
        assert!(ExitStatus::with_code(127).is_missing_program());
        assert!(ExitStatus::with_code(126).is_missing_program());
        assert!(!ExitStatus::with_code(1).is_missing_program());
        assert!(!ExitStatus::success().is_missing_program());
        let runtime = ExitStatus {
            message: Some(
                "OCI runtime exec failed: exec: \"tar\": executable file not found in $PATH".into(),
            ),
            ..ExitStatus::default()
        };
        assert!(runtime.is_missing_program());
        assert!(error_is_missing_program(&OxiError::conflict(
            "exec: \"tar\": Executable file not found in $PATH"
        )));
        assert!(!error_is_missing_program(&OxiError::forbidden(
            "pods/exec is forbidden"
        )));
    }
}
