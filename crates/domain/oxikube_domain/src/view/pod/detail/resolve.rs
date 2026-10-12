//! Turning a container's `env` and `envFrom` into rows, given the ConfigMaps and Secrets they
//! name. The lookups are the caller's (`oxikube_app::workloads` reads each object once); this is
//! pure, so every case is a unit test.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::env::{EnvFrom, EnvSource, EnvVar, SourceKind};
use super::fields::{field_value, resource_value};
use super::secret::SecretValue;
use crate::json::JsonRef;
use crate::resource::Resource;

/// The text of one key of a ConfigMap or Secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceValue {
    /// A ConfigMap value, shown as it is.
    Plain(String),
    /// A decoded Secret value.
    Secret(SecretValue),
}

/// The keys of a ConfigMap or Secret that was found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceData {
    entries: BTreeMap<String, SourceValue>,
}

impl SourceData {
    /// The `data` (and the key names of `binaryData`) of a ConfigMap. A binary value is shown as
    /// `<binary>`.
    pub fn from_config_map(config_map: &Resource) -> Self {
        let mut entries = BTreeMap::new();
        if let Some(data) = config_map.json().get("data").and_then(JsonRef::as_object) {
            for (key, value) in data {
                entries.insert(
                    key.to_owned(),
                    SourceValue::Plain(value.as_str().unwrap_or_default().to_owned()),
                );
            }
        }
        if let Some(data) = config_map
            .json()
            .get("binaryData")
            .and_then(JsonRef::as_object)
        {
            for key in data.keys() {
                entries
                    .entry(key.to_owned())
                    .or_insert_with(|| SourceValue::Plain("<binary>".to_owned()));
            }
        }
        Self { entries }
    }

    /// The decoded `data` of a Secret.
    pub fn from_secret(secret: &Resource) -> Self {
        let mut entries = BTreeMap::new();
        if let Some(data) = secret.json().get("data").and_then(JsonRef::as_object) {
            for (key, value) in data {
                let encoded = value.as_str().unwrap_or_default();
                entries.insert(
                    key.to_owned(),
                    SourceValue::Secret(SecretValue::from_base64(encoded)),
                );
            }
        }
        Self { entries }
    }

    /// The value of `key`.
    pub fn get(&self, key: &str) -> Option<&SourceValue> {
        self.entries.get(key)
    }

    /// The keys, sorted.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
}

/// What the lookup of one ConfigMap or Secret found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceState {
    /// The object exists.
    Found(SourceData),
    /// There is no such object.
    NotFound,
    /// The object could not be read (no permission, a failed request); the reason is as the port
    /// worded it.
    Unavailable(String),
}

/// The ConfigMaps and Secrets a pod's containers name, as looked up.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvSources {
    states: HashMap<(SourceKind, Arc<str>), SourceState>,
}

impl EnvSources {
    /// Records what the lookup of `name` found.
    pub fn insert(&mut self, kind: SourceKind, name: Arc<str>, state: SourceState) {
        self.states.insert((kind, name), state);
    }

    /// What the lookup of `name` found; `None` when it was not looked up.
    pub fn get(&self, kind: SourceKind, name: &Arc<str>) -> Option<&SourceState> {
        self.states.get(&(kind, name.clone()))
    }
}

/// Why a variable has no value to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Missing {
    /// The ConfigMap or Secret does not exist.
    Object {
        /// ConfigMap or Secret.
        kind: SourceKind,
        /// Its name.
        name: Arc<str>,
        /// `optional: true`: the pod runs without the variable.
        optional: bool,
    },
    /// The object exists and has no such key.
    Key {
        /// ConfigMap or Secret.
        kind: SourceKind,
        /// Its name.
        name: Arc<str>,
        /// The key that is missing.
        key: Arc<str>,
        /// `optional: true`.
        optional: bool,
    },
    /// The object could not be read.
    Unavailable {
        /// ConfigMap or Secret.
        kind: SourceKind,
        /// Its name.
        name: Arc<str>,
        /// Why.
        reason: String,
    },
    /// The pod has no such field or the container no such resource (`status.podIP` of a pod that
    /// is not running yet).
    Unset,
}

/// The value of one variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvValue {
    /// A literal, a ConfigMap value, a pod field: shown as it is.
    Plain(String),
    /// A Secret value: masked until the user reveals it.
    Secret(SecretValue),
    /// No value, and why.
    Missing(Missing),
}

/// Where a row's name came from, for the line under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// `value:` in the spec.
    Literal,
    /// A key of a ConfigMap or Secret (`env` or `envFrom`).
    Source {
        /// ConfigMap or Secret.
        kind: SourceKind,
        /// Its name.
        name: Arc<str>,
        /// The key.
        key: Arc<str>,
    },
    /// A field of the pod.
    Field(Arc<str>),
    /// A resource of a container.
    Resource(Arc<str>),
}

/// One row of a container's environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvRow {
    /// The variable's name (an `envFrom` prefix included).
    pub name: String,
    /// Its value.
    pub value: EnvValue,
    /// Where it came from.
    pub origin: Origin,
}

impl EnvRow {
    /// Whether the value is a Secret's (masked by default).
    pub fn is_secret(&self) -> bool {
        matches!(self.value, EnvValue::Secret(_))
    }
}

/// The rows of one container: every `envFrom` key (sorted, after its prefix), then the `env`
/// entries in order. As in Kubernetes an `env` entry replaces an `envFrom` key of the same name,
/// and a later `envFrom` replaces an earlier one.
pub(super) fn rows(
    env: &[EnvVar],
    env_from: &[EnvFrom],
    pod: &Resource,
    container: &str,
    sources: &EnvSources,
) -> Vec<EnvRow> {
    let mut rows: Vec<EnvRow> = Vec::new();
    let put =
        |row: EnvRow, rows: &mut Vec<EnvRow>| match rows.iter().position(|r| r.name == row.name) {
            Some(at) => rows[at] = row,
            None => rows.push(row),
        };
    for from in env_from {
        match sources.get(from.kind, &from.name) {
            Some(SourceState::Found(data)) => {
                for key in data.keys() {
                    let value = match data.get(key) {
                        Some(SourceValue::Secret(secret)) => EnvValue::Secret(secret.clone()),
                        Some(SourceValue::Plain(text)) => EnvValue::Plain(text.clone()),
                        None => continue,
                    };
                    let row = EnvRow {
                        name: format!("{}{key}", from.prefix),
                        value,
                        origin: Origin::Source {
                            kind: from.kind,
                            name: from.name.clone(),
                            key: Arc::from(key),
                        },
                    };
                    put(row, &mut rows);
                }
            }
            other => {
                // The whole source is the problem: one row stands for its keys.
                let missing = match other {
                    Some(SourceState::Unavailable(reason)) => Missing::Unavailable {
                        kind: from.kind,
                        name: from.name.clone(),
                        reason: reason.clone(),
                    },
                    _ => Missing::Object {
                        kind: from.kind,
                        name: from.name.clone(),
                        optional: from.optional,
                    },
                };
                rows.push(EnvRow {
                    name: format!("{}*", from.prefix),
                    value: EnvValue::Missing(missing),
                    origin: Origin::Source {
                        kind: from.kind,
                        name: from.name.clone(),
                        key: Arc::from("*"),
                    },
                });
            }
        }
    }
    for var in env {
        let (value, origin) = resolve_one(var, pod, container, sources);
        put(
            EnvRow {
                name: var.name.to_string(),
                value,
                origin,
            },
            &mut rows,
        );
    }
    rows
}

fn resolve_one(
    var: &EnvVar,
    pod: &Resource,
    container: &str,
    sources: &EnvSources,
) -> (EnvValue, Origin) {
    match &var.source {
        EnvSource::Literal(text) => (EnvValue::Plain(text.to_string()), Origin::Literal),
        EnvSource::KeyRef {
            kind,
            name,
            key,
            optional,
        } => {
            let origin = Origin::Source {
                kind: *kind,
                name: name.clone(),
                key: key.clone(),
            };
            let value = match sources.get(*kind, name) {
                Some(SourceState::Found(data)) => match data.get(key) {
                    Some(SourceValue::Plain(text)) => EnvValue::Plain(text.clone()),
                    Some(SourceValue::Secret(secret)) => EnvValue::Secret(secret.clone()),
                    None => EnvValue::Missing(Missing::Key {
                        kind: *kind,
                        name: name.clone(),
                        key: key.clone(),
                        optional: *optional,
                    }),
                },
                Some(SourceState::Unavailable(reason)) => EnvValue::Missing(Missing::Unavailable {
                    kind: *kind,
                    name: name.clone(),
                    reason: reason.clone(),
                }),
                _ => EnvValue::Missing(Missing::Object {
                    kind: *kind,
                    name: name.clone(),
                    optional: *optional,
                }),
            };
            (value, origin)
        }
        EnvSource::Field(path) => (
            field_value(pod, path).map_or(EnvValue::Missing(Missing::Unset), EnvValue::Plain),
            Origin::Field(path.clone()),
        ),
        EnvSource::ResourceField {
            container: named,
            resource,
        } => (
            resource_value(pod, named.as_deref().unwrap_or(container), resource)
                .map_or(EnvValue::Missing(Missing::Unset), EnvValue::Plain),
            Origin::Resource(resource.clone()),
        ),
    }
}
