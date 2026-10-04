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

/// The content as JSON with unset (`null`) members removed.
///
/// Goes through text rather than `serde_json::to_value`: `to_value` widens an `f32` to `f64`
/// (`0.1_f32` becomes `0.10000000149011612`), while the text serializer prints each float's
/// shortest round-tripping form (`0.1`), which parses back to the `f64` that prints the same.
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
