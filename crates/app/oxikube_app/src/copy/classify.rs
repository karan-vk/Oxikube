//! Maps what a copy dispatch or a namespace call fails with onto [`CopyError`].

use oxikube_domain::{ConflictReason, ErrorKind, OxiError};

use super::refusal::Refusal;
use super::types::{CopyError, CopyRequest};
use crate::apply::FieldConflict;
use crate::command_bus::DispatchError;

pub(super) fn classify_dispatch(request: &CopyRequest, error: DispatchError) -> CopyError {
    match error {
        DispatchError::ReadOnly { context, .. } => CopyError::ReadOnly {
            context: context.to_string(),
        },
        DispatchError::Declined(_) => CopyError::Declined,
        other => classify_oxi(
            &OxiError::from(other),
            &crate::actions::object_label(&request.target),
        ),
    }
}

pub(super) fn classify_oxi(error: &OxiError, label: &str) -> CopyError {
    if let Some(refusal) = Refusal::of(error) {
        return match refusal.clone() {
            Refusal::KindNotServed(message) => CopyError::KindNotServed(message),
            Refusal::VersionNotServed(message) => CopyError::VersionNotServed(message),
            Refusal::NamespaceMissing(namespace) => CopyError::NamespaceMissing(namespace),
            Refusal::SourceUnavailable(message) => CopyError::SourceUnavailable(message),
            Refusal::InvalidName(message) => CopyError::InvalidName(message),
        };
    }
    if let Some(details) = error.conflict_details() {
        match details.reason {
            ConflictReason::FieldOwnership => {
                return CopyError::Conflicts(
                    details
                        .causes
                        .iter()
                        .map(|cause| FieldConflict {
                            field: cause.field.clone(),
                            manager: cause.manager.clone(),
                            applied_at: None,
                        })
                        .collect(),
                );
            }
            ConflictReason::AlreadyExists => return CopyError::Exists(label.to_owned()),
            ConflictReason::StaleVersion | ConflictReason::Other => {}
        }
    }
    if error.kind() == ErrorKind::Validation {
        return CopyError::Invalid {
            message: error.message().to_owned(),
            causes: error
                .validation_details()
                .map(|details| details.causes.clone())
                .unwrap_or_default(),
        };
    }
    CopyError::Failed(error.to_string())
}
