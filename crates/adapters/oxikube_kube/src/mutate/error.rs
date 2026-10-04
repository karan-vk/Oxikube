//! Error mapping for writes: [`classify`] for the kind, plus the structured detail the UI needs.
//!
//! Conflict detail comes from the `Status` the server sends with a 409. A server-side apply
//! clash looks like
//!
//! ```json
//! {"reason": "Conflict", "code": 409,
//!  "message": "Apply failed with 1 conflict: conflict with \"alpha\": .data.k",
//!  "details": {"causes": [{"reason": "FieldManagerConflict", "field": ".data.k",
//!                          "message": "conflict with \"alpha\""}]}}
//! ```
//!
//! and the manager is the first quoted name in a cause's message (the server has no separate
//! field for it; `kubectl` parses it the same way). A stale write has reason `Conflict`, no
//! causes and "the object has been modified" in the message; a create of an existing name has
//! reason `AlreadyExists`.

use kube::core::Status;
use kube::core::response::StatusCause;
use oxikube_domain::{
    ConflictDetails, ConflictReason, ErrorKind, FieldCause, OxiError, ValidationDetails,
};

use crate::auth::{classify, redacted_line};

/// Maps a failed write.
pub(super) fn write_error(err: &kube::Error) -> OxiError {
    let base = classify(err);
    let kube::Error::Api(status) = err else {
        return base;
    };
    match base.kind() {
        ErrorKind::Conflict => base.with_source(conflict_details(status)),
        ErrorKind::Validation => base.with_source(ValidationDetails {
            causes: causes(status).collect(),
        }),
        // 500 and 502 are the server failing the request, not the request being wrong.
        ErrorKind::Internal if status.code >= 500 => base.with_retryable(true),
        _ => base,
    }
}

fn conflict_details(status: &Status) -> ConflictDetails {
    let causes: Vec<FieldCause> = causes(status).collect();
    let reason = if status.reason == "AlreadyExists" {
        ConflictReason::AlreadyExists
    } else if causes.iter().any(|c| c.reason == "FieldManagerConflict") {
        ConflictReason::FieldOwnership
    } else if status.message.contains("the object has been modified") {
        ConflictReason::StaleVersion
    } else {
        ConflictReason::Other
    };
    ConflictDetails {
        reason,
        causes: if reason == ConflictReason::FieldOwnership {
            causes
        } else {
            Vec::new()
        },
    }
}

fn causes(status: &Status) -> impl Iterator<Item = FieldCause> + '_ {
    status
        .details
        .iter()
        .flat_map(|details| details.causes.iter())
        .map(field_cause)
}

fn field_cause(cause: &StatusCause) -> FieldCause {
    let manager = (cause.reason == "FieldManagerConflict")
        .then(|| manager_of(&cause.message))
        .flatten();
    FieldCause {
        field: cause.field.clone(),
        manager,
        reason: cause.reason.clone(),
        message: redacted_line(&cause.message),
    }
}

/// The field manager named in `conflict with "<manager>" using <apiVersion>`.
fn manager_of(message: &str) -> Option<String> {
    let rest = message.strip_prefix("conflict with ")?;
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_owned()).filter(|m| !m.is_empty())
}

#[cfg(test)]
mod unit {
    use super::manager_of;

    #[test]
    fn manager_is_the_first_quoted_name() {
        assert_eq!(
            manager_of("conflict with \"alpha\"").as_deref(),
            Some("alpha")
        );
        assert_eq!(
            manager_of("conflict with \"kubectl-edit\" using apps/v1").as_deref(),
            Some("kubectl-edit")
        );
        assert_eq!(
            manager_of("conflict with \"x\" with subresource \"status\"").as_deref(),
            Some("x")
        );
        assert_eq!(manager_of("conflict with \"\""), None);
        assert_eq!(manager_of("something else"), None);
    }
}
