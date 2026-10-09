//! [`ObjectMeta`] and [`OwnerRef`]: the typed subset of `metadata` every view needs, parsed from
//! a decoded object in one pass.

use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::error::ResourceError;
use super::strmap::StrMap;
use crate::ids::Gvk;
use crate::intern::intern;

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
    pub labels: StrMap,
    /// `metadata.annotations`.
    pub annotations: StrMap,
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
            labels: StrMap::new(),
            annotations: StrMap::new(),
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
    pub(super) fn from_json(meta: &Map<String, Value>) -> Result<Self, ResourceError> {
        let mut out = Self::named("");
        let mut name = None;
        for (key, value) in meta {
            match key.as_str() {
                "name" => name = opt_str(value, "metadata.name")?,
                "namespace" => out.namespace = opt_shared(value, "metadata.namespace")?,
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

/// [`opt_str`] for a text many objects repeat (a namespace): one shared allocation.
fn opt_shared(value: &Value, field: &'static str) -> Result<Option<Arc<str>>, ResourceError> {
    match value {
        Value::String(s) if !s.is_empty() => Ok(Some(intern(s))),
        _ => opt_str(value, field),
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

fn string_map(value: &Value, field: &'static str) -> Result<StrMap, ResourceError> {
    match value {
        Value::Null => Ok(StrMap::new()),
        Value::Object(map) => {
            let pairs = map
                .iter()
                .map(|(k, v)| match v {
                    Value::String(s) => Ok((intern(k), intern(s))),
                    _ => Err(invalid(field, "values must be strings")),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(StrMap::shared(pairs))
        }
        _ => Err(invalid(field, "expected an object")),
    }
}

fn string_list(value: &Value, field: &'static str) -> Result<Vec<Arc<str>>, ResourceError> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(intern(s)),
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
                    .map(intern)
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

/// A required string field of a top-level object.
pub(super) fn req_str<'a>(
    obj: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, ResourceError> {
    match obj.get(field) {
        None | Some(Value::Null) => Err(ResourceError::MissingField { field }),
        Some(Value::String(s)) if s.is_empty() => Err(ResourceError::MissingField { field }),
        Some(Value::String(s)) => Ok(s),
        Some(_) => Err(invalid(field, "expected a string")),
    }
}

pub(super) fn invalid(field: &'static str, reason: &str) -> ResourceError {
    ResourceError::InvalidField {
        field,
        reason: reason.into(),
    }
}
