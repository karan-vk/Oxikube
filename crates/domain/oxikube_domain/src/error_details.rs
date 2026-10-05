//! Structured detail attached to `Conflict` and `Validation` errors of a write.
//!
//! An [`OxiError`] is a kind plus a message. A write that the server rejects can say more:
//! which fields clash with which field manager (a server-side apply conflict, so the UI can
//! offer "force" or "cancel"), or which field paths failed validation (so the editor can put
//! a marker on the line). Adapters attach one of these types as the error's source; callers
//! read it back with [`OxiError::conflict_details`] and [`OxiError::validation_details`]
//! without knowing which adapter produced it.
//!
//! The message text inside a [`FieldCause`] is free text from the server. Adapters redact it
//! before building the detail (the domain never redacts, see [`error`](crate::error)).

use std::error::Error as StdError;
use std::fmt;

use crate::error::{ErrorKind, OxiError};

/// One field-level reason a write was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FieldCause {
    /// The field the server names, in its JSON path notation (`.spec.replicas` for an apply
    /// conflict, `spec.replicas` for a validation failure). Empty when the server named none.
    pub field: String,
    /// The field manager that owns `field`: set for a server-side apply conflict.
    pub manager: Option<String>,
    /// The server's machine-readable reason (`FieldManagerConflict`, `FieldValueInvalid`, ...),
    /// empty when it sent none.
    pub reason: String,
    /// The server's human-readable explanation, redacted and bounded to one line.
    pub message: String,
}

/// Why the server answered HTTP 409.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConflictReason {
    /// Server-side apply without `force` touched fields another manager owns; the causes
    /// name the fields and their owners. Retry with force, or drop those fields.
    FieldOwnership,
    /// The object changed since `metadata.resourceVersion` was read. Re-read, merge, retry.
    StaleVersion,
    /// A create named an object that already exists.
    AlreadyExists,
    /// Any other 409 (for example a delete precondition or a namespace being terminated).
    Other,
}

/// Detail of a `Conflict` write error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictDetails {
    /// What kind of conflict it is.
    pub reason: ConflictReason,
    /// The clashing fields; empty unless `reason` is [`ConflictReason::FieldOwnership`].
    pub causes: Vec<FieldCause>,
}

impl ConflictDetails {
    /// The distinct field managers named by the causes, in first-seen order.
    pub fn managers(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for manager in self.causes.iter().filter_map(|c| c.manager.as_deref()) {
            if !seen.contains(&manager) {
                seen.push(manager);
            }
        }
        seen
    }
}

impl fmt::Display for ConflictDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.reason {
            ConflictReason::FieldOwnership => {
                write!(f, "{} field(s) owned by other managers", self.causes.len())
            }
            ConflictReason::StaleVersion => f.write_str("stale resource version"),
            ConflictReason::AlreadyExists => f.write_str("object already exists"),
            ConflictReason::Other => f.write_str("conflict"),
        }
    }
}

impl StdError for ConflictDetails {}

/// Detail of a `Validation` write error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationDetails {
    /// One cause per rejected field; empty when the server only sent a message.
    pub causes: Vec<FieldCause>,
}

impl fmt::Display for ValidationDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} invalid field(s)", self.causes.len())
    }
}

impl StdError for ValidationDetails {}

impl OxiError {
    /// The [`ConflictDetails`] of a `Conflict` error that carries them.
    pub fn conflict_details(&self) -> Option<&ConflictDetails> {
        if self.kind() != ErrorKind::Conflict {
            return None;
        }
        StdError::source(self)?.downcast_ref::<ConflictDetails>()
    }

    /// The [`ValidationDetails`] of a `Validation` error that carries them.
    pub fn validation_details(&self) -> Option<&ValidationDetails> {
        if self.kind() != ErrorKind::Validation {
            return None;
        }
        StdError::source(self)?.downcast_ref::<ValidationDetails>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cause(field: &str, manager: Option<&str>) -> FieldCause {
        FieldCause {
            field: field.into(),
            manager: manager.map(str::to_owned),
            ..FieldCause::default()
        }
    }

    #[test]
    fn details_round_trip_through_the_error_source() {
        let details = ConflictDetails {
            reason: ConflictReason::FieldOwnership,
            causes: vec![cause(".spec.replicas", Some("kubectl"))],
        };
        let err = OxiError::conflict("apply failed").with_source(details.clone());
        assert_eq!(err.conflict_details(), Some(&details));
        assert!(err.validation_details().is_none());

        let invalid = ValidationDetails {
            causes: vec![cause("spec.replicas", None)],
        };
        let err = OxiError::validation("invalid").with_source(invalid.clone());
        assert_eq!(err.validation_details(), Some(&invalid));
        assert!(err.conflict_details().is_none());
    }

    #[test]
    fn details_are_ignored_on_the_wrong_kind_and_when_absent() {
        let details = ConflictDetails {
            reason: ConflictReason::Other,
            causes: vec![],
        };
        assert!(
            OxiError::internal("x")
                .with_source(details)
                .conflict_details()
                .is_none()
        );
        assert!(OxiError::conflict("x").conflict_details().is_none());
        assert!(OxiError::validation("x").validation_details().is_none());
    }

    #[test]
    fn managers_are_distinct_and_ordered() {
        let details = ConflictDetails {
            reason: ConflictReason::FieldOwnership,
            causes: vec![
                cause(".a", Some("kubectl")),
                cause(".b", Some("helm")),
                cause(".c", Some("kubectl")),
                cause(".d", None),
            ],
        };
        assert_eq!(details.managers(), ["kubectl", "helm"]);
        assert_eq!(details.to_string(), "4 field(s) owned by other managers");
    }
}
