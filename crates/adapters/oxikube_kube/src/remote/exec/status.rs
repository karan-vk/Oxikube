//! The API server's `Status` for a finished command, as an [`ExitStatus`].
//!
//! `kubectl exec` and the kubelet report a command's outcome on the status channel:
//!
//! ```json
//! {"status": "Success"}
//! {"status": "Failure", "reason": "NonZeroExitCode", "message": "command terminated with non-zero exit code: ...",
//!  "details": {"causes": [{"reason": "ExitCode", "message": "3"}]}}
//! ```
//!
//! Any other failure (the executable is missing, the runtime refused) has no exit code: the
//! message is all there is.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use oxikube_ports::ExitStatus;

use crate::auth::redacted_line;

/// The outcome `status` describes.
pub(super) fn exit_status(status: &Status) -> ExitStatus {
    if status.status.as_deref() == Some("Success") {
        return ExitStatus::success();
    }
    let code = status
        .details
        .iter()
        .flat_map(|details| details.causes.iter().flatten())
        .find(|cause| cause.reason.as_deref() == Some("ExitCode"))
        .and_then(|cause| cause.message.as_deref()?.trim().parse().ok());
    let message = status
        .message
        .as_deref()
        .filter(|message| !message.trim().is_empty())
        .map(redacted_line);
    ExitStatus { code, message }
}
