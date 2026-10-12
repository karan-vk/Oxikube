//! A container's environment as the spec writes it: `env` entries and `envFrom` sources, before
//! any ConfigMap or Secret is looked up. [`resolve`](super::resolve) turns them into rows.

use std::sync::Arc;

use crate::json::JsonRef;
use crate::view::{arc_of, arr_of, bool_of, str_of, sub};

/// A ConfigMap or a Secret: the two objects an environment variable can come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceKind {
    /// A ConfigMap: its values are shown.
    ConfigMap,
    /// A Secret: its values are masked until revealed.
    Secret,
}

impl SourceKind {
    /// `ConfigMap` or `Secret`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfigMap => "ConfigMap",
            Self::Secret => "Secret",
        }
    }
}

/// Where an `env` entry's value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvSource {
    /// `value: ...` (empty when the entry has none).
    Literal(Arc<str>),
    /// `valueFrom.configMapKeyRef` or `valueFrom.secretKeyRef`.
    KeyRef {
        /// ConfigMap or Secret.
        kind: SourceKind,
        /// The object's name.
        name: Arc<str>,
        /// The key in its `data`.
        key: Arc<str>,
        /// `optional: true`: a missing object or key is not an error.
        optional: bool,
    },
    /// `valueFrom.fieldRef`: a field of the pod itself (`status.podIP`).
    Field(Arc<str>),
    /// `valueFrom.resourceFieldRef`: a request or limit of a container (`limits.cpu`).
    ResourceField {
        /// The container, the entry's own when `None`.
        container: Option<Arc<str>>,
        /// `limits.cpu`, `requests.memory`...
        resource: Arc<str>,
    },
}

/// One `env` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVar {
    /// The variable's name.
    pub name: Arc<str>,
    /// Where its value comes from.
    pub source: EnvSource,
}

/// One `envFrom` entry: every key of a ConfigMap or Secret becomes a variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvFrom {
    /// ConfigMap or Secret.
    pub kind: SourceKind,
    /// The object's name.
    pub name: Arc<str>,
    /// Put before every key (`prefix`), empty without one.
    pub prefix: Arc<str>,
    /// `optional: true`.
    pub optional: bool,
}

/// The `env` entries of one container spec entry, in order. An entry without a name is skipped.
pub(super) fn env_of(container: JsonRef<'_>) -> Vec<EnvVar> {
    arr_of(container, "env")
        .iter()
        .filter_map(|entry| {
            let name = arc_of(entry, "name")?;
            Some(EnvVar {
                name,
                source: source_of(entry),
            })
        })
        .collect()
}

fn source_of(entry: JsonRef<'_>) -> EnvSource {
    let from = sub(entry, "valueFrom");
    for (key, kind) in [
        ("configMapKeyRef", SourceKind::ConfigMap),
        ("secretKeyRef", SourceKind::Secret),
    ] {
        let reference = sub(from, key);
        if let (Some(name), Some(field)) = (arc_of(reference, "name"), arc_of(reference, "key")) {
            return EnvSource::KeyRef {
                kind,
                name,
                key: field,
                optional: bool_of(reference, "optional"),
            };
        }
    }
    if let Some(path) = arc_of(sub(from, "fieldRef"), "fieldPath") {
        return EnvSource::Field(path);
    }
    let resource = sub(from, "resourceFieldRef");
    if let Some(name) = arc_of(resource, "resource") {
        return EnvSource::ResourceField {
            container: arc_of(resource, "containerName"),
            resource: name,
        };
    }
    EnvSource::Literal(Arc::from(str_of(entry, "value").unwrap_or("")))
}

/// The `envFrom` entries of one container spec entry, in order.
pub(super) fn env_from_of(container: JsonRef<'_>) -> Vec<EnvFrom> {
    arr_of(container, "envFrom")
        .iter()
        .filter_map(|entry| {
            let (kind, reference) = if let Some(r) = entry.get("configMapRef") {
                (SourceKind::ConfigMap, r)
            } else {
                (SourceKind::Secret, entry.get("secretRef")?)
            };
            Some(EnvFrom {
                kind,
                name: arc_of(reference, "name")?,
                prefix: Arc::from(str_of(entry, "prefix").unwrap_or("")),
                optional: bool_of(reference, "optional"),
            })
        })
        .collect()
}
