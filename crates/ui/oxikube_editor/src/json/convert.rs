//! [`convert`]: a buffer's text as the other language, without losing what it says.
//!
//! The conversion is an explicit user action, never a background re-serialisation: the editor
//! calls it on a snapshot off the UI thread and replaces the buffer in one undo step. It is pure
//! (no gpui, no I/O), so it also runs in plain unit tests.
//!
//! # Decisions
//!
//! - **YAML to JSON** reads the buffer with the spanned model, so key order, number digits and
//!   string content are exact. What JSON cannot hold is reported in [`Warnings`]: comments are
//!   dropped (counted), aliases are expanded, tags dropped. A buffer with syntax errors, a key
//!   that is not a scalar, `.inf` or `.nan` is refused with a span.
//! - **Several documents** become one `kind: List` (`apiVersion: v1`, `items`), the Kubernetes
//!   way to put several objects in one JSON document, which `kubectl apply -f` and the apply flow
//!   take. A single document is converted as it is. Going back keeps the `List`: it is not split.
//! - **JSON to YAML** first applies the strict JSON check (a trailing comma is an error, not
//!   silently fine), then writes block-style YAML. Strings that would read as another type stay
//!   quoted. JSON has no comments, so nothing is lost.
//! - JSONC (comments in JSON) is not accepted.

use super::comments::count_comments;
use super::error::ConvertError;
use super::language::Language;
use super::strict::first_error;
use super::value::{Reader, Value};
use super::warnings::Warnings;
use super::write_json::to_json;
use super::write_yaml::to_yaml;
use crate::yaml::{ParseResult, parse};

/// The text of a conversion and what it could not carry over.
#[derive(Clone, Debug, PartialEq)]
pub struct Converted {
    /// The converted text, ending with a newline.
    pub text: String,
    /// The language of [`text`](Self::text).
    pub to: Language,
    /// What was lost or read differently; empty for a faithful conversion.
    pub warnings: Warnings,
}

/// Converts `text`, which is written in the language other than `to`, into `to`.
///
/// # Errors
///
/// A [`ConvertError`] (with a span in `text` when the problem has a place) when the text is
/// empty, is not valid in its language, or holds something the target language cannot say.
pub fn convert(text: &str, to: Language) -> Result<Converted, ConvertError> {
    match to {
        Language::Json => yaml_to_json(text),
        Language::Yaml => json_to_yaml(text),
    }
}

/// YAML (one or more documents) as JSON. See the [module docs](self).
///
/// # Errors
///
/// See [`convert`].
pub fn yaml_to_json(text: &str) -> Result<Converted, ConvertError> {
    let parsed = parse(text);
    refuse_syntax_errors(&parsed, "YAML")?;
    let mut warnings = Warnings {
        comments: count_comments(&parsed),
        ..Warnings::default()
    };
    let mut documents = documents(&parsed, &mut warnings)?;
    let root = match documents.len() {
        0 => return Err(ConvertError::new("there is nothing to convert")),
        1 => documents.remove(0),
        n => {
            warnings.documents = n;
            Value::Map(vec![
                ("apiVersion".to_owned(), Value::Str("v1".to_owned())),
                ("kind".to_owned(), Value::Str("List".to_owned())),
                ("items".to_owned(), Value::Seq(documents)),
            ])
        }
    };
    Ok(Converted {
        text: to_json(&root),
        to: Language::Json,
        warnings,
    })
}

/// JSON as block-style YAML. See the [module docs](self).
///
/// # Errors
///
/// See [`convert`].
pub fn json_to_yaml(text: &str) -> Result<Converted, ConvertError> {
    if text.trim().is_empty() {
        return Err(ConvertError::new("there is nothing to convert"));
    }
    if let Some(error) = first_error(text) {
        return Err(
            ConvertError::at(format!("not valid JSON: {}", error.message), error.span)
                .with_line(text),
        );
    }
    let parsed = parse(text);
    refuse_syntax_errors(&parsed, "JSON")?;
    let mut warnings = Warnings::default();
    let mut documents = documents(&parsed, &mut warnings)?;
    let root = match documents.len() {
        1 => documents.remove(0),
        _ => return Err(ConvertError::new("there is nothing to convert")),
    };
    Ok(Converted {
        text: to_yaml(&root),
        to: Language::Yaml,
        warnings,
    })
}

/// Refuses a parse with syntax errors (they include duplicate keys): converting broken text
/// would convert a guess.
fn refuse_syntax_errors(parsed: &ParseResult, what: &str) -> Result<(), ConvertError> {
    match parsed.diagnostics().first() {
        None => Ok(()),
        Some(first) => {
            let more = parsed.diagnostics().len() - 1;
            let tail = match more {
                0 => String::new(),
                1 => " (and 1 more)".to_owned(),
                n => format!(" (and {n} more)"),
            };
            Err(ConvertError::at(
                format!("fix the {what} first: {}{tail}", first.message),
                first.span.clone(),
            )
            .with_line(parsed.text()))
        }
    }
}

/// The value of every non-empty document of `parsed`.
fn documents(parsed: &ParseResult, warnings: &mut Warnings) -> Result<Vec<Value>, ConvertError> {
    let mut values = Vec::new();
    for doc in parsed.docs() {
        let mut reader = Reader::new(parsed, doc, warnings);
        if let Some(value) = reader.document().map_err(|e| e.with_line(parsed.text()))? {
            values.push(value);
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests;
