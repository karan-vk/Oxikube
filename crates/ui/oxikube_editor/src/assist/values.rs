//! Value completion: the enum members or `true` / `false` of the field whose value is being typed.

use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::{JsonSchema, SchemaType};

use super::complete::{CompletionItem, CompletionKind, rank};
use super::describe::{enum_text, type_label, yaml_scalar};
use super::line::ValueSite;
use super::lookup::{Lookup, schema_at};
use crate::validate::document_gvk;
use crate::yaml::ParseResult;

pub(super) fn complete_value(
    parsed: &ParseResult,
    schema_for: &dyn Fn(&Gvk) -> Option<Arc<JsonSchema>>,
    site: &ValueSite,
) -> Vec<CompletionItem> {
    let Some(doc) = parsed.doc_at(site.key_start) else {
        return Vec::new();
    };
    let Some(key) = parsed.key_at(site.key_start) else {
        return Vec::new();
    };
    if key.key_span.start != site.key_start {
        return Vec::new();
    }
    let Some(schema) = document_gvk(parsed, doc).and_then(|gvk| schema_for(&gvk)) else {
        return Vec::new();
    };
    let Lookup::Known(field) = schema_at(&schema, &key.path.path) else {
        return Vec::new();
    };
    let mut choices: Vec<(String, String)> = field
        .enum_values
        .iter()
        .map(|v| {
            let shown = enum_text(v);
            let insert = if v.is_string() {
                yaml_scalar(&shown)
            } else {
                shown.clone()
            };
            (shown, insert)
        })
        .collect();
    if choices.is_empty() && field.types.contains(&SchemaType::Boolean) {
        choices = ["true", "false"]
            .map(|b| (b.to_owned(), b.to_owned()))
            .to_vec();
    }
    let default = field.default.as_ref().map(enum_text);
    let prefix = site.prefix(parsed.text());
    let lead = if site.space_first { " " } else { "" };
    choices
        .into_iter()
        .filter(|(shown, _)| rank(prefix, shown) == Some(0))
        .map(|(shown, insert)| CompletionItem {
            detail: Some(if default.as_deref() == Some(&shown) {
                format!("{} · default", type_label(field))
            } else {
                type_label(field)
            }),
            documentation: field.description.clone(),
            insert_text: format!("{lead}{insert}"),
            replace: site.replace.clone(),
            required: false,
            kind: CompletionKind::Value,
            label: shown,
        })
        .collect()
}
