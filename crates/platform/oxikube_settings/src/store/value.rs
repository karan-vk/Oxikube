// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/settings/src/settings_store.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! Per-setting storage: the resolved global value, per-cluster values and a change counter.
//!
//! `AnySettingValue` is Zed's type-erased slot (`SettingValue<T>` behind `dyn`), extended
//! with change detection: a recompute bumps the generation only when a resolved value is
//! not equal to the previous one, which is what lets observers skip unrelated reloads.

use std::any::{Any, type_name};
use std::collections::HashMap;
use std::sync::OnceLock;

use schemars::SchemaGenerator;
use serde_json::{Map, Value};

use super::layers::MergedLayers;
use crate::diagnostics::{KeyTree, SettingsDiagnostic};
use crate::schema;
use crate::settings::{Settings, SettingsLocation};

/// Type-erased access to one registered setting.
pub(crate) trait AnySettingValue: Send + Sync {
    /// The setting's key (`None` for root-level content).
    fn key(&self) -> Option<&'static str>;
    /// Rust type name, for diagnostics.
    fn type_name(&self) -> &'static str;
    /// Bumped every time a resolved value changes.
    fn generation(&self) -> u64;
    /// The value for `location`: the cluster's own value if it has overrides, else global.
    fn value_for(&self, location: Option<SettingsLocation>) -> Option<&dyn Any>;
    /// Every cluster that has a value of its own, with that value.
    fn cluster_values(&self) -> Vec<(&str, &dyn Any)>;
    /// Replace the global value (tests, previews); bumps the generation when it differs.
    fn override_global(&mut self, value: Box<dyn Any>);
    /// Re-resolve from `layers`, keeping last good values on type errors.
    fn recompute(&mut self, layers: &MergedLayers, diagnostics: &mut Vec<SettingsDiagnostic>);
    /// Keys the content accepts, from its schema (with inlined subschemas).
    fn key_tree(&self) -> KeyTree;
    /// The content's schema from `generator` (a `$ref` for structs).
    fn json_schema(&self, generator: &mut SchemaGenerator) -> Value;
}

/// Storage for one setting type.
pub(crate) struct SettingValue<T> {
    global: Option<T>,
    clusters: HashMap<String, T>,
    generation: u64,
    /// The content's key tree, built from its schema on first use.
    key_tree: OnceLock<KeyTree>,
}

impl<T> Default for SettingValue<T> {
    fn default() -> Self {
        Self {
            global: None,
            clusters: HashMap::new(),
            generation: 0,
            key_tree: OnceLock::new(),
        }
    }
}

impl<T: Settings> SettingValue<T> {
    fn tree(&self) -> &KeyTree {
        self.key_tree
            .get_or_init(|| KeyTree::from_schema(&schema::inline_schema_for::<T::Content>()))
    }

    /// Whether a cluster's override object says anything about `T`.
    fn overrides_touch(&self, overrides: &Map<String, Value>) -> bool {
        match T::KEY {
            Some(key) => overrides.get(key).is_some_and(|v| !v.is_null()),
            None => match self.tree() {
                KeyTree::Object(fields) => overrides.keys().any(|key| fields.contains_key(key)),
                KeyTree::Any => !overrides.is_empty(),
            },
        }
    }
}

/// Deserialise `T`'s section of `root` and resolve it.
fn resolve<T: Settings>(root: &Value) -> Result<T, String> {
    let section = match T::KEY {
        Some(key) => root.get(key).unwrap_or(&Value::Null),
        None => return deserialize::<T>(root, ""),
    };
    if section.is_null() {
        return Ok(T::from_content(T::Content::default()));
    }
    deserialize::<T>(section, T::KEY.unwrap_or_default())
}

fn deserialize<T: Settings>(section: &Value, key: &str) -> Result<T, String> {
    match serde_path_to_error::deserialize::<_, T::Content>(section) {
        Ok(content) => Ok(T::from_content(content)),
        Err(err) => {
            let path = err.path().to_string();
            let full = match (key.is_empty(), path == ".") {
                (true, _) => path,
                (false, true) => key.to_owned(),
                (false, false) => format!("{key}.{path}"),
            };
            Err(format!("{full}: {}", err.into_inner()))
        }
    }
}

impl<T: Settings> AnySettingValue for SettingValue<T> {
    fn key(&self) -> Option<&'static str> {
        T::KEY
    }

    fn type_name(&self) -> &'static str {
        type_name::<T>()
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn value_for(&self, location: Option<SettingsLocation>) -> Option<&dyn Any> {
        let cluster = location.and_then(|location| self.clusters.get(location.cluster.as_str()));
        cluster
            .or(self.global.as_ref())
            .map(|value| value as &dyn Any)
    }

    fn cluster_values(&self) -> Vec<(&str, &dyn Any)> {
        self.clusters
            .iter()
            .map(|(id, value)| (id.as_str(), value as &dyn Any))
            .collect()
    }

    fn override_global(&mut self, value: Box<dyn Any>) {
        if let Ok(value) = value.downcast::<T>()
            && self.global.as_ref() != Some(&*value)
        {
            self.global = Some(*value);
            self.generation += 1;
        }
    }

    fn recompute(&mut self, layers: &MergedLayers, diagnostics: &mut Vec<SettingsDiagnostic>) {
        let mut changed = false;

        let global = match resolve::<T>(&layers.root) {
            Ok(value) => Some(value),
            Err(message) => {
                diagnostics.push(SettingsDiagnostic::InvalidValue {
                    setting: type_name::<T>(),
                    cluster: None,
                    message,
                });
                // Keep the last good value; on first load fall back to the defaults alone.
                let fallback = match self.global {
                    Some(_) => None,
                    None => Some(resolve::<T>(&layers.defaults).unwrap_or_else(|message| {
                        diagnostics.push(SettingsDiagnostic::InvalidValue {
                            setting: type_name::<T>(),
                            cluster: None,
                            message: format!("default.json: {message}"),
                        });
                        T::from_content(T::Content::default())
                    })),
                };
                let base = fallback.as_ref().or(self.global.as_ref());
                let salvaged = base.and_then(|base| T::salvage(&layers.root, base));
                salvaged.or(fallback)
            }
        };
        if let Some(global) = global
            && self.global.as_ref() != Some(&global)
        {
            self.global = Some(global);
            changed = true;
        }

        let mut previous = std::mem::take(&mut self.clusters);
        for cluster in &layers.clusters {
            if !self.overrides_touch(&cluster.overrides) {
                continue;
            }
            match resolve::<T>(&cluster.merged) {
                Ok(value) => {
                    changed |= previous.get(&cluster.id) != Some(&value);
                    previous.remove(&cluster.id);
                    self.clusters.insert(cluster.id.clone(), value);
                }
                Err(message) => {
                    diagnostics.push(SettingsDiagnostic::InvalidValue {
                        setting: type_name::<T>(),
                        cluster: Some(cluster.id.clone()),
                        message,
                    });
                    // Keep the last good block; with none, the global value stands in. Either
                    // way a setting that must fail closed may be salvaged from the bad block.
                    let last_good = previous.remove(&cluster.id);
                    let base = last_good.as_ref().or(self.global.as_ref());
                    let salvaged = base.and_then(|base| T::salvage(&cluster.merged, base));
                    match (salvaged, last_good) {
                        (Some(value), last_good) => {
                            changed |= last_good.as_ref() != Some(&value);
                            self.clusters.insert(cluster.id.clone(), value);
                        }
                        (None, Some(last_good)) => {
                            self.clusters.insert(cluster.id.clone(), last_good);
                        }
                        (None, None) => {}
                    }
                }
            }
        }
        // Clusters whose overrides went away now read the global value.
        changed |= !previous.is_empty();

        if changed {
            self.generation += 1;
        }
    }

    fn key_tree(&self) -> KeyTree {
        self.tree().clone()
    }

    fn json_schema(&self, generator: &mut SchemaGenerator) -> Value {
        schema::section_schema::<T::Content>(generator)
    }
}
