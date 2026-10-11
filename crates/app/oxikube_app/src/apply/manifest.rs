//! The manifest of a `resource::Apply`: one object read from YAML or JSON text, and the body the
//! requests send.
//!
//! Errors name fields, never values: the text may be a Secret.

use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::{OxiError, OxiResult};
use serde_json::{Map, Value};

use crate::session::ClusterSession;

/// The namespace a namespaced object goes to when neither the manifest nor the command names one
/// (kubectl's default).
pub(crate) const DEFAULT_NAMESPACE: &str = "default";

/// One object to apply: its body and what names it.
#[derive(Debug, Clone)]
pub(crate) struct Manifest {
    /// The object as written (comments and formatting gone).
    object: Value,
    /// `apiVersion` + `kind`.
    pub(crate) gvk: Gvk,
    /// `metadata.name`.
    pub(crate) name: String,
    /// `metadata.namespace`, when the manifest says one.
    pub(crate) namespace: Option<String>,
}

impl Manifest {
    /// Reads `text` (YAML, or JSON, which is YAML) as exactly one object with `apiVersion`, `kind`
    /// and `metadata.name`.
    ///
    /// # Errors
    ///
    /// A `Validation` error for text that is not YAML, that holds no object or several (several
    /// documents are applied one by one, E10-S09), or an object without the fields that name it.
    pub(crate) fn parse(text: &str) -> OxiResult<Self> {
        // Read as kubectl reads it (YAML 1.1): `defaultMode: 0644` is the octal integer 420,
        // not the decimal 644.0 a YAML 1.2 reader makes of it.
        let options = serde_saphyr::options! { legacy_octal_numbers: true };
        // The parser's message quotes the text, which may be a Secret: it is dropped.
        let documents: Vec<Value> = serde_saphyr::from_multiple_with_options(text, options)
            .map_err(|_| OxiError::validation("the manifest is not valid YAML or JSON"))?;
        let mut documents: Vec<Value> = documents.into_iter().filter(|d| !d.is_null()).collect();
        let object = match documents.len() {
            0 => return Err(OxiError::validation("the manifest is empty")),
            1 => documents.remove(0),
            n => {
                return Err(OxiError::validation(format!(
                    "the manifest holds {n} objects; apply one object at a time"
                )));
            }
        };
        let text_at = |pointer: &str| {
            object
                .pointer(pointer)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let missing = |field: &str| OxiError::validation(format!("the manifest has no {field}"));
        if !object.is_object() {
            return Err(OxiError::validation("the manifest is not an object"));
        }
        let api_version = text_at("/apiVersion").ok_or_else(|| missing("apiVersion"))?;
        let kind = text_at("/kind").ok_or_else(|| missing("kind"))?;
        let name = text_at("/metadata/name").ok_or_else(|| missing("metadata.name"))?;
        let namespace = text_at("/metadata/namespace");
        Ok(Self {
            gvk: Gvk::from_api_version(&api_version, &kind),
            name,
            namespace,
            object,
        })
    }

    /// The namespace the object goes to in `session`'s cluster ([`namespace_for`], with what
    /// the cluster's discovery says about the kind; unknown when it cannot say).
    pub(crate) async fn resolve_namespace(
        &self,
        session: &ClusterSession,
        default: Option<&str>,
    ) -> Option<String> {
        let namespaced = match session.discovery() {
            Some(discovery) => discovery
                .resolve(&self.gvk)
                .await
                .ok()
                .flatten()
                .map(|kind| kind.namespaced),
            None => None,
        };
        namespace_for(namespaced, self.namespace.as_deref(), default)
    }

    /// The object `cluster` gets, in `namespace` (`None` for a cluster-scoped kind).
    pub(crate) fn target(&self, cluster: ClusterId, namespace: Option<&str>) -> ResourceRef {
        ResourceRef::new(
            cluster,
            self.gvk.clone(),
            namespace.map(Arc::from),
            self.name.as_str(),
        )
    }

    /// The request body: the object with `metadata.namespace` set to `namespace` (when it is
    /// namespaced), and without `metadata.resourceVersion` and `metadata.managedFields`. A server
    /// side apply rejects `managedFields`, and the field managers, not a version, decide what a
    /// concurrent change means; the replace fallback adds the reviewed version back
    /// ([`with_resource_version`]).
    pub(crate) fn body(&self, namespace: Option<&str>) -> Value {
        let mut body = self.object.clone();
        if let Some(object) = body.as_object_mut() {
            let metadata = object
                .entry("metadata")
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(metadata) = metadata.as_object_mut() {
                metadata.shift_remove("resourceVersion");
                metadata.shift_remove("managedFields");
                if let Some(ns) = namespace {
                    metadata.insert("namespace".into(), Value::String(ns.to_owned()));
                }
            }
        }
        body
    }
}

/// `body` with `metadata.resourceVersion` set: a replace that fails with a conflict when the
/// object changed since that version.
pub(crate) fn with_resource_version(mut body: Value, resource_version: &str) -> Value {
    if let Some(metadata) = body.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.insert(
            "resourceVersion".into(),
            Value::String(resource_version.to_owned()),
        );
    }
    body
}

/// The namespace the object goes to. `namespaced` is what discovery says about the kind (`None`
/// when it could not tell): a cluster-scoped kind has none; a namespaced one takes the manifest's,
/// else `default` (the command's), else [`DEFAULT_NAMESPACE`]; an unknown one takes the manifest's
/// or the command's, and the server decides.
pub(crate) fn namespace_for(
    namespaced: Option<bool>,
    written: Option<&str>,
    default: Option<&str>,
) -> Option<String> {
    let chosen = written.or(default).filter(|ns| !ns.is_empty());
    match namespaced {
        Some(false) => None,
        Some(true) => Some(chosen.unwrap_or(DEFAULT_NAMESPACE).to_owned()),
        None => chosen.map(str::to_owned),
    }
}

/// The object a `resource::Apply` names, for its audit record when the cluster's discovery
/// cannot be asked (no session): the manifest's kind and name, in its namespace or the
/// command's, else in none. With a session the guard asks discovery instead
/// ([`resolved_audit_target`](super::resolved_audit_target)), as the handler does. `None` when
/// the text does not name one object.
pub(crate) fn audit_target(
    cluster: &ClusterId,
    text: &str,
    default: Option<&str>,
) -> Option<ResourceRef> {
    let manifest = Manifest::parse(text).ok()?;
    let namespace = namespace_for(None, manifest.namespace.as_deref(), default);
    Some(manifest.target(cluster.clone(), namespace.as_deref()))
}
