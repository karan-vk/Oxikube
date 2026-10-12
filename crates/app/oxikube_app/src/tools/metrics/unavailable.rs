//! The visible "no metrics" answer.

use oxikube_domain::redact::redact;
use oxikube_ports::{ContentPart, ToolOutput};
use serde_json::json;

/// `metrics unavailable: <reason>` as a normal result: `status: "unavailable"` and the reason,
/// for the model to read and relay. Not an error, and never a zero.
pub(super) fn unavailable(cluster: &str, reason: &str) -> ToolOutput {
    let reason = redact(reason);
    let text = format!("metrics unavailable: {reason}");
    ToolOutput {
        content: vec![ContentPart::text(text)],
        structured: Some(json!({
            "status": "unavailable",
            "cluster": cluster,
            "reason": reason,
        })),
        is_error: false,
    }
}
