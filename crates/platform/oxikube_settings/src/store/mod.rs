// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/settings/src/settings_store.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! [`SettingsStore`]: the layered, typed settings store (a GPUI global).
//!
//! The store keeps the parsed default and user layers, one slot per registered setting type
//! and the diagnostics of the last load. It is plain Rust (no GPUI calls) so the merge rules
//! are unit-tested directly; [`crate::init()`] installs it as a global and wires hot reload.

mod layers;
#[cfg(test)]
mod tests;
mod value;

use std::any::{TypeId, type_name};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use oxikube_domain::{OxiError, OxiResult};
use serde_json::{Map, Value};

use crate::diagnostics::{CLUSTERS_KEY, SCHEMA_KEY};
use crate::diagnostics::{KeyTree, SettingsDiagnostic, collect_unknown_keys};
use crate::jsonc::parse_jsonc_object;
use crate::settings::{RegisteredSetting, Settings, SettingsLocation};
use layers::MergedLayers;
use value::{AnySettingValue, SettingValue};

/// Layered, typed settings: `default.json` → user `settings.json` → `clusters.<id>`.
///
/// Reads ([`SettingsStore::get`]) return references to resolved values and never parse or
/// clone. Writes come from [`SettingsStore::set_user_settings`] (file load, hot reload,
/// [`crate::update_user_settings`]) and re-resolve every setting; a setting whose value did
/// not change keeps its generation, so its observers stay quiet.
pub struct SettingsStore {
    defaults: Map<String, Value>,
    user: Map<String, Value>,
    user_text: Option<String>,
    user_settings_path: Option<PathBuf>,
    layers: MergedLayers,
    values: HashMap<TypeId, Box<dyn AnySettingValue>>,
    /// Registration order, for deterministic iteration.
    order: Vec<TypeId>,
    /// Root keys accepted by the registered settings; rebuilt after a registration.
    root_keys: Option<BTreeMap<String, KeyTree>>,
    diagnostics: Vec<SettingsDiagnostic>,
    /// Sequence number of the newest file write applied by [`crate::update_user_settings`].
    applied_write: u64,
}

impl SettingsStore {
    /// A store over `default_settings` (JSONC) with every setting registered through
    /// [`register_settings!`](crate::register_settings) already loaded.
    ///
    /// Fails only when the defaults are not a JSON object (a build defect: the embedded
    /// `default.json` is checked by tests).
    pub fn new(default_settings: &str) -> OxiResult<Self> {
        let mut store = Self::without_registered(default_settings)?;
        for registered in inventory::iter::<RegisteredSetting>() {
            registered.register_into(&mut store);
        }
        Ok(store)
    }

    /// Like [`SettingsStore::new`] but with no settings registered (tests register their own).
    pub fn without_registered(default_settings: &str) -> OxiResult<Self> {
        let defaults = parse_jsonc_object(default_settings)
            .map_err(|err| OxiError::internal(format!("default settings are invalid: {err}")))?;
        let mut store = Self::empty();
        store.layers = MergedLayers::build(&defaults, &store.user);
        store.defaults = defaults;
        Ok(store)
    }

    /// A store with empty defaults and nothing registered.
    pub fn empty() -> Self {
        let defaults = Map::new();
        let user = Map::new();
        let layers = MergedLayers::build(&defaults, &user);
        Self {
            defaults,
            user,
            user_text: None,
            user_settings_path: None,
            layers,
            values: HashMap::new(),
            order: Vec::new(),
            root_keys: None,
            diagnostics: Vec::new(),
            applied_write: 0,
        }
    }

    /// Add a setting type; a no-op when it is already registered.
    ///
    /// The value is resolved immediately from the current layers.
    pub fn register_setting<T: Settings>(&mut self) {
        let id = TypeId::of::<T>();
        if self.values.contains_key(&id) {
            return;
        }
        debug_assert_ne!(T::KEY, Some(CLUSTERS_KEY), "`clusters` is a reserved key");
        let mut value: Box<dyn AnySettingValue> = Box::new(SettingValue::<T>::default());
        let mut diagnostics = Vec::new();
        value.recompute(&self.layers, &mut diagnostics);
        self.values.insert(id, value);
        self.order.push(id);
        self.root_keys = None;
        // Value diagnostics of other settings are unchanged; unknown keys may now be known.
        self.diagnostics
            .retain(|d| !matches!(d, SettingsDiagnostic::UnknownKey { .. }));
        self.diagnostics.extend(diagnostics);
        let unknown = self.unknown_keys();
        self.diagnostics.extend(unknown);
    }

    /// The value of `T` for `location` (`None`: the global value).
    ///
    /// # Panics
    /// When `T` is not registered: a wiring bug (the owning crate's `init` did not run).
    #[track_caller]
    pub fn get<T: Settings>(&self, location: Option<SettingsLocation>) -> &T {
        self.try_get(location)
            .unwrap_or_else(|| panic!("setting {} is not registered", type_name::<T>()))
    }

    /// The value of `T` for `location`, or `None` when `T` is not registered.
    pub fn try_get<T: Settings>(&self, location: Option<SettingsLocation>) -> Option<&T> {
        self.values
            .get(&TypeId::of::<T>())?
            .value_for(location)?
            .downcast_ref::<T>()
    }

    /// Replace the global value of `T` until the next reload. Ignored when `T` is not
    /// registered.
    pub fn override_global<T: Settings>(&mut self, value: T) {
        if let Some(slot) = self.values.get_mut(&TypeId::of::<T>()) {
            slot.override_global(Box::new(value));
        }
    }

    /// Change counter of `T`: bumped whenever its global or any cluster value changes.
    /// `0` for an unregistered setting.
    pub fn generation<T: Settings>(&self) -> u64 {
        self.values
            .get(&TypeId::of::<T>())
            .map_or(0, |value| value.generation())
    }

    /// Replace the user layer with `text` (JSONC) and re-resolve every setting.
    ///
    /// Invalid JSON keeps the last good user layer and returns a `Validation` error (also
    /// recorded in [`SettingsStore::diagnostics`]). Type errors and unknown keys do not fail
    /// the load; they are recorded as diagnostics. Loading the same text twice is a no-op.
    pub fn set_user_settings(&mut self, text: &str) -> OxiResult<()> {
        if self.user_text.as_deref() == Some(text) {
            return match self.diagnostics.first() {
                Some(SettingsDiagnostic::InvalidJson { message }) => Err(OxiError::validation(
                    format!("settings.json is invalid: {message}"),
                )),
                _ => Ok(()),
            };
        }
        self.user_text = Some(text.to_owned());
        match parse_jsonc_object(text) {
            Ok(user) => {
                self.user = user;
                self.recompute_all();
                Ok(())
            }
            Err(message) => {
                self.diagnostics = vec![SettingsDiagnostic::InvalidJson {
                    message: message.clone(),
                }];
                Err(OxiError::validation(format!(
                    "settings.json is invalid: {message}"
                )))
            }
        }
    }

    /// Replace the default layer (normally the embedded `default.json`).
    pub fn set_default_settings(&mut self, text: &str) -> OxiResult<()> {
        self.defaults = parse_jsonc_object(text)
            .map_err(|err| OxiError::validation(format!("default settings are invalid: {err}")))?;
        self.recompute_all();
        Ok(())
    }

    /// Problems found by the last load (syntax errors, type errors, unknown keys).
    pub fn diagnostics(&self) -> &[SettingsDiagnostic] {
        &self.diagnostics
    }

    /// The last good user layer as parsed JSON.
    pub fn raw_user_settings(&self) -> &Map<String, Value> {
        &self.user
    }

    /// The text last passed to [`SettingsStore::set_user_settings`], valid or not.
    pub fn user_settings_text(&self) -> Option<&str> {
        self.user_text.as_deref()
    }

    /// The user `settings.json` this store loads from, if it is backed by a file.
    pub fn user_settings_path(&self) -> Option<&Path> {
        self.user_settings_path.as_deref()
    }

    /// Apply the text of file write number `seq`, unless a newer write was applied already.
    pub(crate) fn apply_written_text(&mut self, seq: u64, text: &str) -> OxiResult<()> {
        if seq <= self.applied_write {
            return Ok(());
        }
        self.applied_write = seq;
        self.set_user_settings(text)
    }

    pub(crate) fn set_user_settings_path(&mut self, path: Option<PathBuf>) {
        self.user_settings_path = path;
    }

    /// JSON schema of `settings.json` for every registered setting (see [`crate::schema`]).
    pub fn json_schema(&self) -> Value {
        let mut generator = crate::schema::generator();
        let mut sections: Vec<_> = self
            .order
            .iter()
            .filter_map(|id| self.values.get(id))
            .map(|value| {
                (
                    value.key(),
                    value.type_name(),
                    value.json_schema(&mut generator),
                )
            })
            .collect();
        sections.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        crate::schema::assemble(sections, generator)
    }

    fn recompute_all(&mut self) {
        self.layers = MergedLayers::build(&self.defaults, &self.user);
        let mut diagnostics = Vec::new();
        for id in &self.order {
            if let Some(value) = self.values.get_mut(id) {
                value.recompute(&self.layers, &mut diagnostics);
            }
        }
        self.diagnostics = diagnostics;
        let unknown = self.unknown_keys();
        self.diagnostics.extend(unknown);
    }

    /// Unknown keys in the user layer and each cluster override.
    fn unknown_keys(&mut self) -> Vec<SettingsDiagnostic> {
        let root_keys = self.root_keys.get_or_insert_with(|| {
            let mut keys = BTreeMap::new();
            for value in self.order.iter().filter_map(|id| self.values.get(id)) {
                match (value.key(), value.key_tree()) {
                    (Some(key), tree) => {
                        keys.insert(key.to_owned(), tree);
                    }
                    (None, KeyTree::Object(fields)) => keys.extend(fields),
                    // Root content we cannot enumerate: accept every key.
                    (None, KeyTree::Any) => return BTreeMap::new(),
                }
            }
            keys.insert(SCHEMA_KEY.to_owned(), KeyTree::Any);
            keys
        });
        let mut out = Vec::new();
        if root_keys.is_empty() && !self.order.is_empty() {
            return out;
        }
        let user_settings: Map<String, Value> = self
            .user
            .iter()
            .filter(|(key, _)| key.as_str() != CLUSTERS_KEY)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        collect_unknown_keys(&user_settings, root_keys, "", &mut out);
        if let Some(Value::Object(clusters)) = self.user.get(CLUSTERS_KEY) {
            for (id, overrides) in clusters {
                if let Value::Object(overrides) = overrides {
                    let prefix = format!("{CLUSTERS_KEY}.{id}");
                    collect_unknown_keys(overrides, root_keys, &prefix, &mut out);
                }
            }
        }
        out
    }
}

impl std::fmt::Debug for SettingsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsStore")
            .field(
                "settings",
                &self
                    .order
                    .iter()
                    .filter_map(|id| self.values.get(id))
                    .map(|value| value.type_name())
                    .collect::<Vec<_>>(),
            )
            .field("user_settings_path", &self.user_settings_path)
            .field("diagnostics", &self.diagnostics)
            .finish_non_exhaustive()
    }
}
