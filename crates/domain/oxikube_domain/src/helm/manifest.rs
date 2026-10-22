//! Splitting a rendered release manifest into the objects it declares (E16-S03).
//!
//! `helm get manifest` prints the objects of a release as YAML documents separated by `---`,
//! each usually preceded by a `# Source: chart/templates/x.yaml` comment. This module is the pure
//! part of "manifest -> resources": it finds the documents, reads
//! `{apiVersion, kind, metadata.name, metadata.namespace}` of each and sets aside hooks. Which
//! kinds are namespaced, and joining with live objects, belong to the app layer.
//!
//! * Empty and comment-only documents are skipped.
//! * A `kind: List` document contributes its `items`.
//! * One broken document is reported as a [`ManifestError`] and does not drop the others.
//! * Hooks (`helm.sh/hook` annotation) are listed apart in [`ParsedManifest::hooks`]: they are
//!   not live resources of the release.
//!
//! # Sensitivity
//!
//! A manifest can embed credentials (Secret `stringData`). Nothing here keeps document text:
//! [`ManifestObject`] holds identity only and [`ManifestError`] holds a position, never a snippet.

use serde_json::Value;

/// The annotation that marks a hook.
const HOOK_ANNOTATION: &str = "helm.sh/hook";

/// One object a manifest declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestObject {
    /// `apiVersion` as written (`apps/v1`, `v1`).
    pub api_version: String,
    /// `kind` as written.
    pub kind: String,
    /// `metadata.name`.
    pub name: String,
    /// `metadata.namespace` when the manifest sets one (otherwise the release namespace applies,
    /// for a namespaced kind).
    pub namespace: Option<String>,
    /// The template file the object came from (`# Source:` comment), when present.
    pub source: Option<String>,
}

impl ManifestObject {
    /// The API group: the part of `apiVersion` before `/`, empty for the core group.
    pub fn group(&self) -> &str {
        self.api_version.split_once('/').map_or("", |(g, _)| g)
    }

    /// The API version: the part of `apiVersion` after `/`.
    pub fn version(&self) -> &str {
        self.api_version
            .split_once('/')
            .map_or(self.api_version.as_str(), |(_, v)| v)
    }
}

/// A document that could not be read. Carries a position, never a piece of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    /// 1-based index among the non-empty documents.
    pub document: usize,
    /// The template file the document came from, when known.
    pub source: Option<String>,
    /// What is wrong, in words, without document content.
    pub message: String,
}

/// The result of [`parse_manifest`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedManifest {
    /// The release's live objects, in manifest order.
    pub objects: Vec<ManifestObject>,
    /// Hook objects (`helm.sh/hook`), in manifest order.
    pub hooks: Vec<ManifestObject>,
    /// Documents that failed to parse or lack `kind`, `apiVersion` or `metadata.name`.
    pub errors: Vec<ManifestError>,
}

/// One YAML document with the `# Source:` comment that preceded it.
struct Document<'a> {
    text: String,
    source: Option<&'a str>,
}

/// Splits `manifest` at lines that start with `---`, dropping documents with no content.
fn split(manifest: &str) -> Vec<Document<'_>> {
    let mut docs = Vec::new();
    let mut current = String::new();
    let mut source = None;
    let mut has_content = false;
    for line in manifest.lines() {
        let is_separator = line
            .strip_prefix("---")
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']));
        if is_separator {
            if has_content {
                docs.push(Document {
                    text: std::mem::take(&mut current),
                    source,
                });
            }
            current.clear();
            has_content = false;
            source = None;
            continue;
        }
        let trimmed = line.trim();
        if let Some(path) = trimmed.strip_prefix("# Source:") {
            if !has_content {
                source = Some(path.trim());
            }
            continue;
        }
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            has_content = true;
        }
        current.push_str(line);
        current.push('\n');
    }
    if has_content {
        docs.push(Document {
            text: current,
            source,
        });
    }
    docs
}

/// `" at line N column M"` out of a YAML error's text, or an empty string. Only digits are
/// copied, so no document content can come along.
fn location(error: &str) -> String {
    let number_after = |marker: &str| {
        let tail = &error[error.find(marker)? + marker.len()..];
        let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
        (!digits.is_empty()).then_some(digits)
    };
    match (number_after("line "), number_after("column ")) {
        (Some(line), Some(column)) => format!(" at line {line} column {column}"),
        (Some(line), None) => format!(" at line {line}"),
        _ => String::new(),
    }
}

fn text<'v>(value: &'v Value, path: &[&str]) -> Option<&'v str> {
    path.iter()
        .try_fold(value, |v, key| v.get(key))
        .and_then(Value::as_str)
}

/// Reads one object value into `parsed`, or an error entry for `document`.
fn take(value: &Value, document: usize, source: Option<&str>, parsed: &mut ParsedManifest) {
    let object = (|| {
        Some(ManifestObject {
            api_version: text(value, &["apiVersion"])?.to_owned(),
            kind: text(value, &["kind"])?.to_owned(),
            name: text(value, &["metadata", "name"])?.to_owned(),
            namespace: text(value, &["metadata", "namespace"])
                .filter(|ns| !ns.is_empty())
                .map(str::to_owned),
            source: source.map(str::to_owned),
        })
    })();
    let Some(object) = object else {
        parsed.errors.push(ManifestError {
            document,
            source: source.map(str::to_owned),
            message: "the document has no apiVersion, kind and metadata.name".into(),
        });
        return;
    };
    let is_hook = value
        .get("metadata")
        .and_then(|m| m.get("annotations"))
        .and_then(|a| a.get(HOOK_ANNOTATION))
        .is_some();
    if is_hook {
        parsed.hooks.push(object);
    } else {
        parsed.objects.push(object);
    }
}

/// Parses the rendered manifest of a release. Never fails as a whole; see the module docs.
///
/// Cost is linear in the manifest size; run it off the UI thread for large releases.
pub fn parse_manifest(manifest: &str) -> ParsedManifest {
    let mut parsed = ParsedManifest::default();
    for (index, doc) in split(manifest).into_iter().enumerate() {
        let document = index + 1;
        let value: Value = match serde_saphyr::from_str(&doc.text) {
            Ok(value) => value,
            Err(error) => {
                parsed.errors.push(ManifestError {
                    document,
                    source: doc.source.map(str::to_owned),
                    message: format!("invalid YAML{}", location(&error.to_string())),
                });
                continue;
            }
        };
        if value.is_null() {
            continue;
        }
        match value.get("items").and_then(Value::as_array) {
            Some(items) if text(&value, &["kind"]).is_some_and(|k| k.ends_with("List")) => {
                for item in items {
                    take(item, document, doc.source, &mut parsed);
                }
            }
            _ => take(&value, document, doc.source, &mut parsed),
        }
    }
    parsed
}

#[cfg(test)]
mod tests;
