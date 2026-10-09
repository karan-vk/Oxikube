//! The thin [`Resource`] model: [`ObjectMeta`] + [`Gvk`] + the object's JSON, held compactly.
//!
//! Kubernetes has 200+ kinds and arbitrary CRDs, so the domain does not model
//! them one by one (ADR 0005). A [`Resource`] keeps the handful of metadata
//! fields every screen needs in a typed [`ObjectMeta`] and the whole object as
//! a [`JsonDoc`]: the JSON in one compact byte buffer, shared between clones and
//! read in place (E07-P603; a `serde_json::Value` tree cost 13 to 17 KB per pod).
//! CRDs and unknown kinds therefore work with no extra code, and `k8s-openapi`
//! never reaches the domain.
//!
//! Field reads go through the JSON-pointer accessors ([`Resource::get`],
//! [`Resource::get_str`], [`Resource::get_i64`], [`Resource::get_bool`]) or the root
//! view ([`Resource::json`]); they borrow from the document and never allocate. A full
//! [`Value`] tree is built only on request ([`Resource::to_value`]).
//!
//! # Secrets
//!
//! The JSON of a `Secret` holds base64 `data`. This module has no
//! special case for it; anything that logs, audits or persists a `Resource`
//! must first pass it through the redaction path. Never print a `Resource`
//! with `{:?}` in a log line.

mod error;
mod meta;
mod strmap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::Gvk;
use crate::json::{JsonDoc, JsonRef};

pub use error::ResourceError;
pub use meta::{ObjectMeta, OwnerRef};
pub use strmap::StrMap;

use meta::{invalid, req_str};

/// A Kubernetes object: typed metadata, its [`Gvk`], and the JSON.
///
/// The JSON is kept as received (the workspace enables `serde_json`'s
/// `preserve_order`, so key order survives for the YAML view). `meta` and `kind`
/// are derived from it by [`Resource::from_json`]; if you change the JSON
/// (through [`Resource::edit_json`]), rebuild the `Resource` to keep them in sync.
///
/// Cloning is cheap: the clone shares the document (an `Arc` of bytes) and copies only `meta`.
///
/// The JSON may hold base64 Secret data. See the [module docs](self#secrets).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    /// Typed metadata.
    pub meta: ObjectMeta,
    /// Group-version-kind from `apiVersion` + `kind`.
    pub kind: Gvk,
    /// The object as received: complete, unless [`partial`](Self::partial) is set.
    ///
    /// Shared, not copied, when the `Resource` is cloned: a watch feed's cache and the
    /// resource store hold the same bytes. Read through [`json`](Self::json).
    #[serde(rename = "json")]
    doc: JsonDoc,
    /// Set on metadata-only objects (`PartialObjectMetadata`): the JSON then holds `apiVersion`,
    /// `kind` and `metadata` but no `spec`, `status` or data, so a view must not render it as a
    /// complete object. Read through [`is_partial`](Self::is_partial); set by
    /// [`into_partial`](Self::into_partial). A full `get` of the same object replaces it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub partial: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl std::fmt::Debug for Resource {
    /// Prints identity only, never the JSON, so a stray `{:?}` cannot leak Secret data.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resource")
            .field("kind", &self.kind)
            .field("namespace", &self.meta.namespace)
            .field("name", &self.meta.name)
            .finish_non_exhaustive()
    }
}

impl Resource {
    /// Build a `Resource` from a decoded Kubernetes object, encoding its JSON compactly (the
    /// `Value` tree is dropped).
    ///
    /// Reads `apiVersion`, `kind` and `metadata` in one pass over `metadata`.
    ///
    /// # Errors
    ///
    /// [`ResourceError::NotAnObject`] for non-object input,
    /// [`ResourceError::MissingField`] when `apiVersion`, `kind` or `metadata`
    /// is absent, [`ResourceError::MissingName`] when `metadata.name` is absent
    /// or empty, and [`ResourceError::InvalidField`] for wrongly typed fields.
    pub fn from_json(json: Value) -> Result<Self, ResourceError> {
        let obj = json.as_object().ok_or(ResourceError::NotAnObject)?;
        let api_version = req_str(obj, "apiVersion")?;
        let kind = req_str(obj, "kind")?;
        let meta = match obj.get("metadata") {
            None | Some(Value::Null) => {
                return Err(ResourceError::MissingField { field: "metadata" });
            }
            Some(Value::Object(m)) => ObjectMeta::from_json(m)?,
            Some(_) => {
                return Err(invalid("metadata", "expected an object"));
            }
        };
        let kind = Gvk::from_api_version(api_version, kind);
        Ok(Self {
            meta,
            kind,
            doc: JsonDoc::from_value(&json),
            partial: false,
        })
    }

    /// Marks this resource as metadata-only (see [`partial`](Self::partial)).
    #[must_use]
    pub fn into_partial(mut self) -> Self {
        self.partial = true;
        self
    }

    /// Whether this holds metadata only: `spec`, `status` and data were not fetched.
    pub fn is_partial(&self) -> bool {
        self.partial
    }

    /// `metadata.name`.
    pub fn name(&self) -> &str {
        &self.meta.name
    }

    /// `metadata.namespace`; `None` for cluster-scoped objects.
    pub fn namespace(&self) -> Option<&str> {
        self.meta.namespace.as_deref()
    }

    /// The object's JSON document (shared between clones).
    pub fn doc(&self) -> &JsonDoc {
        &self.doc
    }

    /// The root of the object's JSON: the whole object as a borrowed, read-only view.
    pub fn json(&self) -> JsonRef<'_> {
        self.doc.root()
    }

    /// The value at a JSON pointer (RFC 6901), for example `/spec/replicas`.
    ///
    /// The empty pointer returns the whole object. Returns `None` when the path
    /// does not exist.
    pub fn get(&self, pointer: &str) -> Option<JsonRef<'_>> {
        self.doc.root().pointer(pointer)
    }

    /// The string at `pointer`; `None` if missing or not a string.
    pub fn get_str(&self, pointer: &str) -> Option<&str> {
        self.get(pointer)?.as_str()
    }

    /// The integer at `pointer`; `None` if missing or not representable as `i64`.
    pub fn get_i64(&self, pointer: &str) -> Option<i64> {
        self.get(pointer)?.as_i64()
    }

    /// The boolean at `pointer`; `None` if missing or not a boolean.
    pub fn get_bool(&self, pointer: &str) -> Option<bool> {
        self.get(pointer)?.as_bool()
    }

    /// The object as a [`Value`] tree. Allocates the whole tree (several times the size of the
    /// document), so it is for cold paths: editing, schemas, a one-off export.
    pub fn to_value(&self) -> Value {
        self.doc.to_value()
    }

    /// Changes the JSON with `edit`, which sees the full tree, and stores the result. `meta` and
    /// `kind` are not updated; rebuild the `Resource` if the edit touches them. Other clones keep
    /// the old document.
    pub fn edit_json(&mut self, edit: impl FnOnce(&mut Value)) {
        let mut value = self.doc.to_value();
        edit(&mut value);
        self.doc = JsonDoc::from_value(&value);
    }

    /// Remove `metadata.managedFields` from the JSON, leaving everything else (and
    /// key order) untouched. `meta` is unaffected. Returns whether anything was
    /// removed.
    pub fn strip_managed_fields(&mut self) -> bool {
        let has = self
            .get("/metadata")
            .and_then(JsonRef::as_object)
            .is_some_and(|m| m.contains_key("managedFields"));
        if !has {
            return false;
        }
        let mut removed = false;
        self.edit_json(|json| {
            removed = json
                .get_mut("metadata")
                .and_then(Value::as_object_mut)
                .is_some_and(|m| m.shift_remove("managedFields").is_some());
        });
        removed
    }

    /// Render the JSON as YAML.
    ///
    /// The output is the object exactly as held; call
    /// [`strip_managed_fields`](Self::strip_managed_fields) first for a
    /// kubectl-style view. Strings that YAML 1.1 readers would resolve to
    /// another type (`y`, `no`, `on`, `1e3`, `null`, ...) are emitted quoted so
    /// they round-trip as strings.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Yaml`] if the serializer fails.
    pub fn to_yaml(&self) -> Result<String, ResourceError> {
        serde_saphyr::to_string(&self.doc).map_err(|e| ResourceError::Yaml(e.to_string()))
    }
}
