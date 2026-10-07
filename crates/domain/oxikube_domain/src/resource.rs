//! The thin [`Resource`] model: [`ObjectMeta`] + [`Gvk`] + the raw JSON object.
//!
//! Kubernetes has 200+ kinds and arbitrary CRDs, so the domain does not model
//! them one by one (ADR 0005). A [`Resource`] keeps the handful of metadata
//! fields every screen needs in a typed [`ObjectMeta`] and the whole object as
//! an untouched [`serde_json::Value`], shared between clones. CRDs and unknown
//! kinds therefore work with no extra code, and `k8s-openapi` never reaches the
//! domain.
//!
//! Field reads go through the JSON-pointer accessors ([`Resource::get`],
//! [`Resource::get_str`], [`Resource::get_i64`], [`Resource::get_bool`]); they
//! are thin wrappers over [`Value::pointer`] and never allocate.
//!
//! # Secrets
//!
//! [`Resource::json`] of a `Secret` holds base64 `data`. This module has no
//! special case for it; anything that logs, audits or persists a `Resource`
//! must first pass it through the redaction path. Never print a `Resource`
//! with `{:?}` in a log line.

use std::collections::BTreeMap;
use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::OxiError;
use crate::ids::Gvk;

/// Why a JSON value could not become a [`Resource`], or a [`Resource`] could not
/// be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResourceError {
    /// The input was not a JSON object.
    #[error("resource must be a JSON object")]
    NotAnObject,
    /// A required top-level field (`apiVersion`, `kind`, `metadata`) is absent.
    #[error("resource is missing `{field}`")]
    MissingField {
        /// JSON path of the absent field.
        field: &'static str,
    },
    /// `metadata.name` is absent or empty.
    #[error("resource is missing `metadata.name`")]
    MissingName,
    /// A field is present but has the wrong type or an unparsable value.
    #[error("invalid `{field}`: {reason}")]
    InvalidField {
        /// JSON path of the offending field.
        field: &'static str,
        /// What was wrong with it.
        reason: String,
    },
    /// YAML serialisation failed.
    #[error("yaml serialisation failed: {0}")]
    Yaml(String),
}

impl From<ResourceError> for OxiError {
    fn from(err: ResourceError) -> Self {
        OxiError::validation(err.to_string())
    }
}

/// One entry of `metadata.ownerReferences`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OwnerRef {
    /// `apiVersion` of the owner, for example `apps/v1`.
    pub api_version: Arc<str>,
    /// Kind of the owner, for example `ReplicaSet`.
    pub kind: Arc<str>,
    /// Name of the owner (same namespace, or cluster-scoped).
    pub name: Arc<str>,
    /// UID of the owner.
    pub uid: Arc<str>,
    /// Whether this owner is the managing controller.
    pub controller: bool,
    /// Whether the owner cannot be deleted before this object.
    pub block_owner_deletion: bool,
}

impl OwnerRef {
    /// The owner's [`Gvk`], derived from `api_version` and `kind`.
    pub fn gvk(&self) -> Gvk {
        Gvk::from_api_version(&self.api_version, &self.kind)
    }
}

/// The typed subset of `metadata` that every view needs.
///
/// All text fields are `Arc<str>`, so cloning is a refcount bump per field. The
/// maps are ordered by key, which gives stable display and snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMeta {
    /// `metadata.name`. Always non-empty.
    pub name: Arc<str>,
    /// `metadata.namespace`; `None` for cluster-scoped objects.
    pub namespace: Option<Arc<str>>,
    /// `metadata.uid`.
    pub uid: Option<Arc<str>>,
    /// `metadata.resourceVersion`. Opaque; compare for equality only.
    pub resource_version: Option<Arc<str>>,
    /// `metadata.labels`.
    pub labels: BTreeMap<Arc<str>, Arc<str>>,
    /// `metadata.annotations`.
    pub annotations: BTreeMap<Arc<str>, Arc<str>>,
    /// `metadata.ownerReferences`.
    pub owner_refs: Vec<OwnerRef>,
    /// `metadata.finalizers`.
    pub finalizers: Vec<Arc<str>>,
    /// `metadata.creationTimestamp`.
    pub creation: Option<Timestamp>,
    /// `metadata.deletionTimestamp`; set while the object is terminating.
    pub deletion: Option<Timestamp>,
}

impl ObjectMeta {
    /// A bare `ObjectMeta` with only a name; every other field empty.
    pub fn named(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            namespace: None,
            uid: None,
            resource_version: None,
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
            owner_refs: Vec::new(),
            finalizers: Vec::new(),
            creation: None,
            deletion: None,
        }
    }

    /// Whether the object is terminating (`deletionTimestamp` is set).
    pub fn is_terminating(&self) -> bool {
        self.deletion.is_some()
    }

    /// The owner flagged `controller: true`, if any.
    pub fn controller_ref(&self) -> Option<&OwnerRef> {
        self.owner_refs.iter().find(|o| o.controller)
    }

    /// Parse the `metadata` object in one pass.
    fn from_json(meta: &Map<String, Value>) -> Result<Self, ResourceError> {
        let mut out = Self::named("");
        let mut name = None;
        for (key, value) in meta {
            match key.as_str() {
                "name" => name = opt_str(value, "metadata.name")?,
                "namespace" => out.namespace = opt_str(value, "metadata.namespace")?,
                "uid" => out.uid = opt_str(value, "metadata.uid")?,
                "resourceVersion" => {
                    out.resource_version = opt_str(value, "metadata.resourceVersion")?;
                }
                "labels" => out.labels = string_map(value, "metadata.labels")?,
                "annotations" => out.annotations = string_map(value, "metadata.annotations")?,
                "ownerReferences" => out.owner_refs = owner_refs(value)?,
                "finalizers" => out.finalizers = string_list(value, "metadata.finalizers")?,
                "creationTimestamp" => {
                    out.creation = opt_timestamp(value, "metadata.creationTimestamp")?;
                }
                "deletionTimestamp" => {
                    out.deletion = opt_timestamp(value, "metadata.deletionTimestamp")?;
                }
                _ => {}
            }
        }
        out.name = name.ok_or(ResourceError::MissingName)?;
        Ok(out)
    }
}

/// A Kubernetes object: typed metadata, its [`Gvk`], and the raw JSON.
///
/// `json` is kept exactly as received (the workspace enables `serde_json`'s
/// `preserve_order`, so key order survives for the YAML view). `meta` and `kind`
/// are derived from it by [`Resource::from_json`]; if you mutate `json`
/// (through [`Resource::json_mut`]), rebuild the `Resource` to keep them in sync.
///
/// Cloning is cheap: the clone shares `json` (an [`Arc`]) and copies only `meta`.
///
/// `json` may hold base64 Secret data. See the [module docs](self#secrets).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    /// Typed metadata.
    pub meta: ObjectMeta,
    /// Group-version-kind from `apiVersion` + `kind`.
    pub kind: Gvk,
    /// The object as received: complete, unless [`partial`](Self::partial) is set.
    ///
    /// Shared, not copied, when the `Resource` is cloned: a watch feed's cache and the
    /// resource store hold the same tree (a `Value` tree costs several times its JSON, so a
    /// second copy of 10 000 pods is about 200 MB; #508). Mutate through
    /// [`json_mut`](Self::json_mut), which copies the tree only while it is shared.
    pub json: Arc<Value>,
    /// Set on metadata-only objects (`PartialObjectMetadata`): `json` then holds `apiVersion`,
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
    /// Prints identity only, never `json`, so a stray `{:?}` cannot leak Secret data.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resource")
            .field("kind", &self.kind)
            .field("namespace", &self.meta.namespace)
            .field("name", &self.meta.name)
            .finish_non_exhaustive()
    }
}

impl Resource {
    /// Build a `Resource` from a decoded Kubernetes object, taking ownership of
    /// the value without cloning it.
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
                return Err(ResourceError::InvalidField {
                    field: "metadata",
                    reason: "expected an object".into(),
                });
            }
        };
        let kind = Gvk::from_api_version(api_version, kind);
        Ok(Self {
            meta,
            kind,
            json: Arc::new(json),
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

    /// The value at a JSON pointer (RFC 6901), for example `/spec/replicas`.
    ///
    /// The empty pointer returns the whole object. Returns `None` when the path
    /// does not exist.
    pub fn get(&self, pointer: &str) -> Option<&Value> {
        self.json.pointer(pointer)
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

    /// Remove `metadata.managedFields` from `json`, leaving everything else (and
    /// key order) untouched. `meta` is unaffected. Returns whether anything was
    /// removed.
    pub fn strip_managed_fields(&mut self) -> bool {
        let has = self
            .json
            .get("metadata")
            .and_then(Value::as_object)
            .is_some_and(|m| m.contains_key("managedFields"));
        has && self
            .json_mut()
            .get_mut("metadata")
            .and_then(Value::as_object_mut)
            .is_some_and(|m| m.shift_remove("managedFields").is_some())
    }

    /// The JSON for editing: copied first if another clone of this `Resource` shares it
    /// ([`Arc::make_mut`]), so the change never shows through those clones. `meta` and `kind`
    /// are not updated; rebuild the `Resource` if the edit touches them.
    pub fn json_mut(&mut self) -> &mut Value {
        Arc::make_mut(&mut self.json)
    }

    /// The JSON by value, without a copy when this is the only holder of the tree.
    pub fn into_json(self) -> Value {
        Arc::unwrap_or_clone(self.json)
    }

    /// Render `json` as YAML.
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
        serde_saphyr::to_string(&self.json).map_err(|e| ResourceError::Yaml(e.to_string()))
    }
}

fn req_str<'a>(obj: &'a Map<String, Value>, field: &'static str) -> Result<&'a str, ResourceError> {
    match obj.get(field) {
        None | Some(Value::Null) => Err(ResourceError::MissingField { field }),
        Some(Value::String(s)) if s.is_empty() => Err(ResourceError::MissingField { field }),
        Some(Value::String(s)) => Ok(s),
        Some(_) => Err(invalid(field, "expected a string")),
    }
}

fn invalid(field: &'static str, reason: &str) -> ResourceError {
    ResourceError::InvalidField {
        field,
        reason: reason.into(),
    }
}

/// `null` and the empty string both mean "unset".
fn opt_str(value: &Value, field: &'static str) -> Result<Option<Arc<str>>, ResourceError> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) if s.is_empty() => Ok(None),
        Value::String(s) => Ok(Some(Arc::from(s.as_str()))),
        _ => Err(invalid(field, "expected a string")),
    }
}

fn opt_timestamp(value: &Value, field: &'static str) -> Result<Option<Timestamp>, ResourceError> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) => s
            .parse::<Timestamp>()
            .map(Some)
            .map_err(|e| invalid(field, &e.to_string())),
        _ => Err(invalid(field, "expected an RFC 3339 string")),
    }
}

fn string_map(
    value: &Value,
    field: &'static str,
) -> Result<BTreeMap<Arc<str>, Arc<str>>, ResourceError> {
    match value {
        Value::Null => Ok(BTreeMap::new()),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| match v {
                Value::String(s) => Ok((Arc::from(k.as_str()), Arc::from(s.as_str()))),
                _ => Err(invalid(field, "values must be strings")),
            })
            .collect(),
        _ => Err(invalid(field, "expected an object")),
    }
}

fn string_list(value: &Value, field: &'static str) -> Result<Vec<Arc<str>>, ResourceError> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(Arc::from(s.as_str())),
                _ => Err(invalid(field, "items must be strings")),
            })
            .collect(),
        _ => Err(invalid(field, "expected an array")),
    }
}

fn owner_refs(value: &Value) -> Result<Vec<OwnerRef>, ResourceError> {
    const FIELD: &str = "metadata.ownerReferences";
    let items = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(items) => items,
        _ => return Err(invalid(FIELD, "expected an array")),
    };
    items
        .iter()
        .map(|item| {
            let obj = item
                .as_object()
                .ok_or_else(|| invalid(FIELD, "items must be objects"))?;
            let text = |key: &str| -> Result<Arc<str>, ResourceError> {
                obj.get(key)
                    .and_then(Value::as_str)
                    .map(Arc::from)
                    .ok_or_else(|| invalid(FIELD, "apiVersion, kind, name and uid are required"))
            };
            let flag = |key: &str| obj.get(key).and_then(Value::as_bool).unwrap_or(false);
            Ok(OwnerRef {
                api_version: text("apiVersion")?,
                kind: text("kind")?,
                name: text("name")?,
                uid: text("uid")?,
                controller: flag("controller"),
                block_owner_deletion: flag("blockOwnerDeletion"),
            })
        })
        .collect()
}
