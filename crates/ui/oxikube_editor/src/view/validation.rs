//! One validation pass over the buffer: the spanned YAML model (E10-S02) and the schema
//! validator (E10-S03), turned into the editor's diagnostics. Pure, so it runs on the background
//! executor and in plain tests. The view runs the same pass a chunk at a time
//! ([`super::progressive`]); [`validate_text`] runs it in one go.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;
use oxikube_ui::editor::EditorDiagnostic;

use super::progressive::{Coverage, Progress, check_chunk, plan};
use crate::cancel::CancelToken;
use crate::json::{Language, LanguageMode};
use crate::yaml::parse_shared;

/// How long typing must pause before the buffer is validated again.
pub const VALIDATION_DEBOUNCE: Duration = Duration::from_millis(150);

/// What the editor knows about each kind's schema: `Some` when the cluster served one, `None`
/// when it has none (or it could not be fetched). A kind missing from the map is not known yet.
pub type KnownSchemas = HashMap<Gvk, Option<Arc<JsonSchema>>>;

/// The outcome of a validation pass, or of its first chunks.
#[derive(Clone, Debug, PartialEq)]
pub struct Validation {
    /// The buffer version that was validated; a result for an older version is dropped.
    pub version: u64,
    /// How the language was chosen for this pass; a result for another mode (the user switched
    /// meanwhile) is dropped.
    pub mode: LanguageMode,
    /// The language the buffer was read as: the [`mode`](Self::mode)'s, or what the content
    /// looked like under [`LanguageMode::Auto`].
    pub language: Language,
    /// Syntax errors and the schema findings of every object checked so far whose schema is
    /// known.
    pub diagnostics: Vec<EditorDiagnostic>,
    /// Kinds the buffer declares whose schema is not known yet: fetch them and validate again.
    pub missing: Vec<Gvk>,
    /// How much of the buffer the diagnostics cover; complete for a one-shot pass.
    pub coverage: Coverage,
}

/// Validates `text` (the buffer at `version`) against the `known` schemas, reading it as YAML or
/// JSON by its content ([`LanguageMode::Auto`]); see [`validate_text_in`].
pub fn validate_text(version: u64, text: Arc<str>, known: &KnownSchemas) -> Validation {
    validate_text_in(version, text, known, LanguageMode::Auto, false)
}

/// [`validate_text`], and when `masking` (the buffer is a Secret opened with its values masked,
/// E10-S06), an error on each placeholder still in it: it must never be applied as a value.
pub fn validate_text_with(
    version: u64,
    text: Arc<str>,
    known: &KnownSchemas,
    masking: bool,
) -> Validation {
    validate_text_in(version, text, known, LanguageMode::Auto, masking)
}

/// Validates `text` (the buffer at `version`) against the `known` schemas, read as the language
/// `mode` gives; see [`validate_text_in`].
pub fn validate_text_as(
    version: u64,
    text: Arc<str>,
    known: &KnownSchemas,
    mode: LanguageMode,
) -> Validation {
    validate_text_in(version, text, known, mode, false)
}

/// The whole pass in one go: `text` read as the language `mode` gives, and when `masking` the
/// placeholder check of [`validate_text_with`].
///
/// JSON goes through the same pipeline as YAML (JSON is YAML flow style: the same spanned model,
/// the same schema diagnostics, `kind: List` per item), plus the strict JSON check
/// ([`crate::json::first_error`]): its finding replaces the YAML parser's syntax errors, which
/// would accept a trailing comma or single quotes and describe other breakage in YAML terms.
pub fn validate_text_in(
    version: u64,
    text: Arc<str>,
    known: &KnownSchemas,
    mode: LanguageMode,
    masking: bool,
) -> Validation {
    let plan = plan(version, Arc::new(parse_shared(text)), None, mode, masking);
    let mut progress = Progress::start(&plan);
    let never = CancelToken::new();
    for chunk in &plan.chunks {
        if progress.is_complete() {
            break;
        }
        if let Ok(findings) = check_chunk(&plan.parsed, chunk, known, progress.room(), &never) {
            progress.add(findings);
        }
    }
    progress.validation()
}

#[cfg(test)]
mod tests {
    use oxikube_domain::schema::JsonSchema;
    use oxikube_ui::editor::DiagnosticLevel;
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
