//! A server-side apply whose manifest does not fit the kind's schema (E10-S13, found on kind).
//!
//! The API server reads an apply patch into the kind's structured schema before anything else,
//! and when that fails it answers `500` with the reason only in the message, no `causes`:
//!
//! ```text
//! failed to create typed patch object (ns/w; test.oxikube.dev/v1, Kind=Widget): .spec.colour: field not declared in schema
//! failed to create typed patch object (ns/d; apps/v1, Kind=Deployment): errors:
//!   .spec.replicas: expected numeric (int or float), got string
//!   .spec.bogus: field not declared in schema
//! ```
//!
//! The manifest is wrong, not the server, and a retry fails the same way. [`typed_patch_error`]
//! makes it a `Validation` error with one cause per field named, in the server's field notation
//! (`.spec.containers[name="web"].image`), so the editor puts each on its line as it does the
//! causes of a `422`. Messages pass through the redaction module like every other cause.

use kube::core::Status;
use oxikube_domain::{FieldCause, OxiError, ValidationDetails};

use crate::auth::redacted_line;

/// How the server's message starts.
const PREFIX: &str = "failed to create typed patch object";

/// The reason of each cause: the value at the field is not what the schema allows (an unknown
/// field included).
const REASON: &str = "FieldValueInvalid";

/// The `Validation` error for a typed-patch failure, `None` for any other status.
pub(super) fn typed_patch_error(status: &Status) -> Option<OxiError> {
    let causes = typed_patch_causes(&status.message)?;
    Some(
        OxiError::validation(redacted_line(&status.message))
            .with_source(ValidationDetails { causes }),
    )
}

/// One cause per `.<path>: <message>` of a typed-patch failure message; `None` when `message`
/// is not one, or names no field.
fn typed_patch_causes(message: &str) -> Option<Vec<FieldCause>> {
    let rest = message.trim_start().strip_prefix(PREFIX)?;
    // The object in parentheses (`ns/name; group/version, Kind=K`) never holds "): ".
    let (_, reasons) = rest.split_once("): ")?;
    let causes: Vec<FieldCause> = reasons
        .strip_prefix("errors:")
        .unwrap_or(reasons)
        .lines()
        .filter_map(cause)
        .collect();
    (!causes.is_empty()).then_some(causes)
}

/// `.spec.colour: field not declared in schema` as a cause.
fn cause(line: &str) -> Option<FieldCause> {
    let line = line.trim();
    if !line.starts_with('.') {
        return None;
    }
    let (field, message) = split_field(line)?;
    Some(FieldCause {
        field: field.to_owned(),
        manager: None,
        reason: REASON.to_owned(),
        message: redacted_line(message),
    })
}

/// Splits `<field>: <message>` at the first `": "` outside a bracket (a list item's key value,
/// `[name="a: b"]`, may hold one).
fn split_field(line: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' if depth > 0 => quoted = !quoted,
            '[' if !quoted => depth += 1,
            ']' if !quoted => depth = depth.saturating_sub(1),
            ':' if depth == 0 && line[i..].starts_with(": ") => {
                return Some((&line[..i], line[i + 2..].trim()));
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod unit {
    use super::typed_patch_causes;

    fn fields(message: &str) -> Vec<(String, String)> {
        typed_patch_causes(message)
            .unwrap_or_default()
            .into_iter()
            .map(|c| (c.field, c.message))
            .collect()
    }

    #[test]
    fn one_field_follows_the_object() {
        assert_eq!(
            fields(
                "failed to create typed patch object (ns/w; test.oxikube.dev/v1, Kind=Widget): \
                 .spec.colour: field not declared in schema"
            ),
            [(
                ".spec.colour".to_owned(),
                "field not declared in schema".to_owned()
            )]
        );
    }

    #[test]
    fn several_fields_come_one_per_line() {
        assert_eq!(
            fields(
                "failed to create typed patch object (ns/d; apps/v1, Kind=Deployment): errors:\n  \
                 .spec.replicas: expected numeric (int or float), got string\n  \
                 .spec.bogus: field not declared in schema"
            ),
            [
                (
                    ".spec.replicas".to_owned(),
                    "expected numeric (int or float), got string".to_owned()
                ),
                (
                    ".spec.bogus".to_owned(),
                    "field not declared in schema".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn a_list_item_key_may_hold_a_colon() {
        assert_eq!(
            fields(
                "failed to create typed patch object (ns/p; v1, Kind=Pod): \
                 .spec.containers[name=\"a: b\"].imag: field not declared in schema"
            ),
            [(
                ".spec.containers[name=\"a: b\"].imag".to_owned(),
                "field not declared in schema".to_owned()
            )]
        );
    }

    #[test]
    fn other_messages_are_not_typed_patch_failures() {
        assert_eq!(typed_patch_causes("etcdserver: request timed out"), None);
        assert_eq!(
            typed_patch_causes("failed to create typed patch object (ns/a; v1, Kind=ConfigMap)"),
            None,
            "no field named"
        );
        assert_eq!(
            typed_patch_causes(
                "failed to create typed patch object (ns/a; v1, Kind=ConfigMap): something broke"
            ),
            None
        );
    }
}
