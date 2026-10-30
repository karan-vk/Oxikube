// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/settings/src/settings_store.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! Typed, comment-preserving edits of a settings file (Zed's `edits_for_update`).
//!
//! The caller mutates a setting's typed content; the old and new content are serialised and
//! diffed with [`update_value_in_json_text`], which rewrites only the changed values. Keys
//! the content type does not know (other settings, unknown keys, comments) are never in
//! either value, so they are left exactly as they were.

#[cfg(test)]
mod tests;

use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use serde_json::Value;

use crate::diagnostics::CLUSTERS_KEY;
use crate::json_edit::{infer_json_indent_size, update_value_in_json_text};
use crate::jsonc::{parse_jsonc_object, strip_nulls};
use crate::settings::Settings;

/// The text of a settings file after `update` changed `T`'s content.
///
/// `cluster` selects the layer: `None` edits the root of the file, `Some(id)` edits
/// `clusters.<id>`. Setting a content field to `None` removes it from the file.
///
/// Fails with `Validation` when `old_text` is not valid JSONC or `T`'s current section has a
/// type error (the user fixes the file first; it is never rewritten blindly), and with
/// `Internal` if the edit would produce unparsable text.
pub fn new_text_for_update<T: Settings>(
    old_text: &str,
    cluster: Option<&ClusterId>,
    update: impl FnOnce(&mut T::Content),
) -> OxiResult<String> {
    let root = parse_jsonc_object(old_text).map_err(|err| {
        OxiError::validation(format!(
            "settings.json could not be parsed; fix it before editing: {err}"
        ))
    })?;
    let root = Value::Object(root);

    let mut key_path: Vec<&str> = Vec::new();
    let mut section = &root;
    if let Some(cluster) = cluster {
        key_path.extend([CLUSTERS_KEY, cluster.as_str()]);
        section = section
            .get(CLUSTERS_KEY)
            .and_then(|clusters| clusters.get(cluster.as_str()))
            .unwrap_or(&Value::Null);
    }
    if let Some(key) = T::KEY {
        key_path.push(key);
        section = section.get(key).unwrap_or(&Value::Null);
    }

    let mut content: T::Content = if section.is_null() {
        T::Content::default()
    } else {
        serde_path_to_error::deserialize(section).map_err(|err| {
            OxiError::validation(format!(
                "settings.json has an invalid value at {}; fix it before editing: {}",
                err.path(),
                err.inner()
            ))
        })?
    };
    let old_value = content_value::<T>(&content)?;
    update(&mut content);
    let new_value = content_value::<T>(&content)?;

    let mut text = old_text.to_owned();
    let tab_size = infer_json_indent_size(&text);
    let mut edits = Vec::new();
    update_value_in_json_text(
        &mut text,
        &mut key_path,
        tab_size,
        &old_value,
        &new_value,
        &mut edits,
    );

    parse_jsonc_object(&text)
        .map_err(|err| OxiError::internal(format!("settings edit produced invalid JSON: {err}")))?;
    Ok(text)
}

/// The text of a settings file after the value at `key_path` was set to `value`.
///
/// The untyped counterpart of [`new_text_for_update`], for a UI driven by the settings schema
/// rather than by a Rust type: the path is the JSON key path (`["terminal", "font_size"]`),
/// `cluster` prefixes it with `clusters.<id>`. Missing intermediate objects are created, an
/// intermediate that is not an object is replaced by one, and `Value::Null` resets the field:
/// the key is removed, along with any parent object the removal leaves empty. Comments,
/// key order and unrelated formatting survive.
///
/// Fails with `Validation` when `old_text` is not valid JSONC or `key_path` is empty, and with
/// `Internal` if the edit would produce unparsable text.
pub fn new_text_for_path_update(
    old_text: &str,
    cluster: Option<&ClusterId>,
    key_path: &[&str],
    value: Value,
) -> OxiResult<String> {
    let full_path: Vec<&str> = cluster
        .map(|id| [CLUSTERS_KEY, id.as_str()])
        .into_iter()
        .flatten()
        .chain(key_path.iter().copied())
        .collect();
    if key_path.is_empty() {
        return Err(OxiError::validation("a settings path must not be empty"));
    }
    let root = parse_jsonc_object(old_text).map_err(|err| {
        OxiError::validation(format!(
            "settings.json could not be parsed; fix it before editing: {err}"
        ))
    })?;
    let old_value = Value::Object(root);
    let mut new_value = old_value.clone();
    set_at_path(&mut new_value, &full_path, value);

    let mut text = old_text.to_owned();
    let tab_size = infer_json_indent_size(&text);
    let mut edits = Vec::new();
    update_value_in_json_text(
        &mut text,
        &mut Vec::new(),
        tab_size,
        &old_value,
        &new_value,
        &mut edits,
    );

    parse_jsonc_object(&text)
        .map_err(|err| OxiError::internal(format!("settings edit produced invalid JSON: {err}")))?;
    Ok(text)
}

/// Set `value` at `path` inside `root`, or remove it (pruning emptied parents) for `Null`.
fn set_at_path(root: &mut Value, path: &[&str], value: Value) {
    let Some((key, rest)) = path.split_first() else {
        *root = value;
        return;
    };
    if !root.is_object() {
        if value.is_null() {
            return;
        }
        *root = Value::Object(serde_json::Map::new());
    }
    let Some(object) = root.as_object_mut() else {
        return;
    };
    if rest.is_empty() && value.is_null() {
        object.remove(*key);
        return;
    }
    if value.is_null() && !object.contains_key(*key) {
        return;
    }
    let child = object.entry((*key).to_owned()).or_insert(Value::Null);
    set_at_path(child, rest, value);
    if value_is_empty_object(child) && !rest.is_empty() {
        object.remove(*key);
    }
}

fn value_is_empty_object(value: &Value) -> bool {
    value.as_object().is_some_and(serde_json::Map::is_empty)
}

/// The content as JSON with unset (`null`) members removed.
///
/// Goes through text rather than `serde_json::to_value`: `to_value` widens an `f32` to `f64`
/// (`0.1_f32` becomes `0.10000000149011612`), while the text serializer prints each float's
/// shortest round-tripping form (`0.1`), which parses back to the `f64` that prints the same.
/// The parse is exact for `f64` fields too because the workspace enables serde_json's
/// `float_roundtrip` feature (without it, 17-digit values drift by one ULP).
fn content_value<T: Settings>(content: &T::Content) -> OxiResult<Value> {
    serde_json::to_string(content)
        .and_then(|text| serde_json::from_str::<Value>(&text))
        .map(|value| strip_nulls(&value))
        .map_err(|err| {
            OxiError::internal(format!(
                "could not serialise {}: {err}",
                std::any::type_name::<T>()
            ))
        })
}
