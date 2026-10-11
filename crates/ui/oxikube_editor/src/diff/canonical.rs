//! One writer for both sides of a diff, and the buffer read as values.
//!
//! The buffer is parsed into JSON values (comments and formatting are gone), so the diff never
//! shows a comment or a re-indented block as a change. Both sides are then written by the same
//! YAML writer (the domain's, `serde-saphyr`) with sorted keys, so key order never shows either.

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;
use serde_json::Value;

use super::normalize::{
    LAST_APPLIED, SECRET_FIELDS, SECRET_PLACEHOLDER, is_secret, normalize, sort_keys,
};

/// Why a buffer could not be read as manifests. Carries no buffer text, so it is safe to log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The buffer is not valid YAML (the editor's problems say where).
    Syntax,
    /// The buffer holds no document.
    Empty,
    /// A document is not an object with `apiVersion`, `kind` and `metadata.name`.
    NotAnObject,
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ReadError::Syntax => "the manifest is not valid YAML",
            ReadError::Empty => "the manifest is empty",
            ReadError::NotAnObject => "the manifest has no apiVersion, kind and metadata.name",
        })
    }
}

/// The documents of `text` as JSON values (empty documents skipped). Comments, anchors and
/// formatting are gone.
///
/// # Errors
///
/// [`ReadError::Syntax`] when the text is not YAML (the parser's message is dropped: it quotes
/// the text, which may hold Secret data), [`ReadError::Empty`] when it holds no document.
pub fn read_documents(text: &str) -> Result<Vec<Value>, ReadError> {
    let documents: Vec<Value> = serde_saphyr::from_multiple(text).map_err(|_| ReadError::Syntax)?;
    let documents: Vec<Value> = documents.into_iter().filter(|d| !d.is_null()).collect();
    if documents.is_empty() {
        return Err(ReadError::Empty);
    }
    Ok(documents)
}

/// The object a manifest document names: kind, namespace and name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ObjectId {
    /// `apiVersion` + `kind`.
    pub gvk: Gvk,
    /// `metadata.namespace`, when the document says one.
    pub namespace: Option<Arc<str>>,
    /// `metadata.name`.
    pub name: Arc<str>,
}

impl ObjectId {
    /// The object `document` names.
    ///
    /// # Errors
    ///
    /// [`ReadError::NotAnObject`] without `apiVersion`, `kind` or `metadata.name`.
    pub fn of(document: &Value) -> Result<Self, ReadError> {
        fn text(value: Option<&Value>) -> Option<&str> {
            value.and_then(Value::as_str).filter(|s| !s.is_empty())
        }
        let api_version = text(document.get("apiVersion")).ok_or(ReadError::NotAnObject)?;
        let kind = text(document.get("kind")).ok_or(ReadError::NotAnObject)?;
        let metadata = document.get("metadata");
        let name = text(metadata.and_then(|m| m.get("name"))).ok_or(ReadError::NotAnObject)?;
        let namespace = text(metadata.and_then(|m| m.get("namespace")));
        Ok(Self {
            gvk: Gvk::from_api_version(api_version, kind),
            namespace: namespace.map(Arc::from),
            name: Arc::from(name),
        })
    }

    /// Whether `resource` is this object (a missing namespace matches any).
    pub fn is(&self, resource: &Resource) -> bool {
        resource.kind.group == self.gvk.group
            && resource.kind.kind == self.gvk.kind
            && resource.name() == &*self.name
            && self
                .namespace
                .as_deref()
                .is_none_or(|ns| resource.namespace() == Some(ns))
    }
}

/// The document of `text` to compare with `live`: the only one, or the one naming `live`.
///
/// # Errors
///
/// What [`read_documents`] fails with, or [`ReadError::NotAnObject`] when several documents
/// name none of them `live`.
pub fn document_for(text: &str, live: Option<&Resource>) -> Result<Value, ReadError> {
    let mut documents = read_documents(text)?;
    if documents.len() == 1 {
        return Ok(documents.remove(0));
    }
    let live = live.ok_or(ReadError::NotAnObject)?;
    documents
        .into_iter()
        .find(|d| ObjectId::of(d).is_ok_and(|id| id.is(live)))
        .ok_or(ReadError::NotAnObject)
}

/// `value` written as YAML with the writer every side of a diff uses. `value`'s key order is
/// kept: sort it first ([`sort_keys`]) for a canonical text.
pub fn to_yaml(value: &Value) -> String {
    // The writer does not fail on JSON values; an empty text compares as "everything changed".
    serde_saphyr::to_string(value).unwrap_or_default()
}

/// `value` as the diff shows it: [`normalize`]d against `reference`, then written.
pub fn normalized_text(value: &Value, reference: Option<&Value>) -> String {
    to_yaml(&normalize(value, reference))
}

/// The text a buffer loaded from `resource` holds (reload, merge): the object with
/// `metadata.managedFields` removed, keys sorted, and a Secret's values masked with
/// [`SECRET_PLACEHOLDER`] (decoded or not, a value never reaches the buffer from here). The
/// open-from-resource renderer (E10-S06) can stand in for it through
/// [`Renderer`](super::Renderer).
pub fn buffer_text(resource: &Resource) -> String {
    let mut value = resource.to_value();
    if let Some(meta) = value.get_mut("metadata").and_then(Value::as_object_mut) {
        meta.shift_remove("managedFields");
    }
    if is_secret(&value)
        && let Some(object) = value.as_object_mut()
    {
        for field in SECRET_FIELDS {
            if let Some(map) = object.get_mut(field).and_then(Value::as_object_mut) {
                for entry in map.values_mut() {
                    *entry = Value::String(SECRET_PLACEHOLDER.to_owned());
                }
            }
        }
        if let Some(annotations) = object
            .get_mut("metadata")
            .and_then(|m| m.get_mut("annotations"))
            .and_then(Value::as_object_mut)
        {
            annotations.shift_remove(LAST_APPLIED);
        }
    }
    to_yaml(&sort_keys(value))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn documents_drop_comments_and_empty_documents() {
        let docs = read_documents("# lead\na: 1 # trailing\n---\n---\nb: [x]\n").unwrap();
        assert_eq!(docs, vec![json!({"a": 1}), json!({"b": ["x"]})]);
        assert_eq!(read_documents("# only a comment\n"), Err(ReadError::Empty));
        assert_eq!(read_documents("a: [\n"), Err(ReadError::Syntax));
    }

    #[test]
    fn an_object_id_needs_kind_and_name() {
        let id = ObjectId::of(&json!({
            "apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": "web", "namespace": "shop"}
        }))
        .unwrap();
        assert_eq!(id.gvk, Gvk::new("apps", "v1", "Deployment"));
        assert_eq!(id.namespace.as_deref(), Some("shop"));
        assert_eq!(
            ObjectId::of(&json!({"kind": "Pod"})),
            Err(ReadError::NotAnObject)
        );
    }

    #[test]
    fn the_buffer_text_hides_managed_fields_and_secret_values() {
        let secret = Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "s", "managedFields": [{"manager": "kubectl"}]},
            "data": {"k": "c2VjcmV0"}
        }))
        .unwrap();
        let text = buffer_text(&secret);
        assert!(!text.contains("managedFields"), "{text}");
        assert!(!text.contains("c2VjcmV0"), "{text}");
        assert!(text.contains(SECRET_PLACEHOLDER), "{text}");
    }
}
