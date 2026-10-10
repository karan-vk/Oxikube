//! One validation pass over the buffer: the spanned YAML model (E10-S02) and the schema
//! validator (E10-S03), turned into the editor's diagnostics. Pure, so it runs on the background
//! executor and in plain tests.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;
use oxikube_ui::editor::{DiagnosticLevel, EditorDiagnostic};

use crate::validate::{self, Severity, ValidateOptions};
use crate::yaml::parse_shared;

/// How long typing must pause before the buffer is validated again.
pub const VALIDATION_DEBOUNCE: Duration = Duration::from_millis(150);

/// What the editor knows about each kind's schema: `Some` when the cluster served one, `None`
/// when it has none (or it could not be fetched). A kind missing from the map is not known yet.
pub type KnownSchemas = HashMap<Gvk, Option<Arc<JsonSchema>>>;

/// The outcome of [`validate_text`].
#[derive(Clone, Debug, PartialEq)]
pub struct Validation {
    /// The buffer version that was validated; a result for an older version is dropped.
    pub version: u64,
    /// Syntax errors and the schema findings of every document whose schema is known.
    pub diagnostics: Vec<EditorDiagnostic>,
    /// Kinds the buffer declares whose schema is not known yet: fetch them and validate again.
    pub missing: Vec<Gvk>,
}

/// Validates `text` (the buffer at `version`) against the `known` schemas.
pub fn validate_text(version: u64, text: Arc<str>, known: &KnownSchemas) -> Validation {
    let parsed = parse_shared(text);
    let mut missing: Vec<Gvk> = Vec::new();
    let found = validate::validate_buffer(
        &parsed,
        |gvk| match known.get(gvk) {
            Some(schema) => schema.clone(),
            None => {
                if !missing.contains(gvk) {
                    missing.push(gvk.clone());
                }
                None
            }
        },
        &ValidateOptions::default(),
    );
    Validation {
        version,
        diagnostics: found.iter().map(to_editor).collect(),
        missing,
    }
}

/// The editor's form of a validator finding.
fn to_editor(found: &validate::Diagnostic) -> EditorDiagnostic {
    let level = match found.severity {
        Severity::Error => DiagnosticLevel::Error,
        Severity::Warning => DiagnosticLevel::Warning,
    };
    EditorDiagnostic::new(found.span.clone(), level, found.message.clone())
        .with_code(found.code.as_str())
}

#[cfg(test)]
mod tests {
    use oxikube_domain::schema::JsonSchema;
    use serde_json::json;

    use super::*;

    fn configmap_schema() -> Arc<JsonSchema> {
        Arc::new(JsonSchema::from_value(&json!({
            "type": "object",
            "properties": {
                "apiVersion": {"type": "string"},
                "kind": {"type": "string"},
                "data": {"type": "object", "additionalProperties": {"type": "string"}}
            }
        })))
    }

    #[test]
    fn syntax_errors_need_no_schema_and_unknown_kinds_are_asked_for() {
        let text: Arc<str> = Arc::from("apiVersion: v1\nkind: ConfigMap\ndata: [\n");
        let result = validate_text(3, text, &KnownSchemas::new());
        assert_eq!(result.version, 3);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("syntax") && d.level == DiagnosticLevel::Error)
        );
        assert_eq!(result.missing, vec![Gvk::new("", "v1", "ConfigMap")]);
    }

    #[test]
    fn a_known_schema_reports_findings_with_spans() {
        let text = "apiVersion: v1\nkind: ConfigMap\nmetdata: {}\n";
        let mut known = KnownSchemas::new();
        known.insert(Gvk::new("", "v1", "ConfigMap"), Some(configmap_schema()));
        let result = validate_text(1, Arc::from(text), &known);
        assert!(result.missing.is_empty());
        let unknown = result
            .diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("unknown-field"))
            .expect("unknown field");
        assert_eq!(&text[unknown.range.clone()], "metdata");
        assert_eq!(unknown.level, DiagnosticLevel::Warning);
    }

    #[test]
    fn a_kind_without_a_schema_is_not_asked_for_again() {
        let mut known = KnownSchemas::new();
        known.insert(Gvk::new("example.com", "v1", "Widget"), None);
        let text = "apiVersion: example.com/v1\nkind: Widget\nspec: {}\n";
        let result = validate_text(1, Arc::from(text), &known);
        assert!(result.missing.is_empty());
        assert!(result.diagnostics.is_empty());
    }
}
