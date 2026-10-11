//! [`render_resource`]: a live object as the text the editor opens with, the way `kubectl get -o
//! yaml` shows it with safer defaults. A pure function over the object (nothing from GPUI), so it
//! runs on the background executor and in plain unit tests.
//!
//! What it does to the object before writing it:
//!
//! * keys are sorted at every level, as kubectl prints them (`apiVersion`, `kind`, `metadata`,
//!   `spec`, `status`, ...); sequences keep their order;
//! * `metadata.managedFields` is removed unless [`RenderOptions::managed_fields`];
//! * a Secret's `data` and `stringData` values are replaced by [`MASKED`] unless
//!   [`RenderOptions::reveal_secrets`], and the `last-applied-configuration` annotation, which
//!   embeds them, is removed with them;
//! * with [`RenderOptions::decode_secrets`] (and the values revealed) a Secret's `data` shows
//!   each value as plaintext, a binary one as a placeholder (`(binary, 12 bytes)`), and the
//!   annotation is removed too: "edit decoded" (E12-S11). The buffer is re-encoded to base64
//!   when it is applied;
//! * `kind: List` objects (and `*List` kinds) are handled item by item.
//!
//! The returned [`RenderedManifest`] says where the hidden and masked parts were and which byte
//! ranges to dim (`status`), so the view can draw them without parsing the text again.

use std::ops::Range;

use oxikube_domain::{Resource, secret};
use serde_json::{Map, Value};

use super::regions;

/// What stands in for a Secret value that is not shown.
pub const MASKED: &str = secret::MASKED_VALUE;

/// What to show of an object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RenderOptions {
    /// Keep `metadata.managedFields` (hidden by default, as `kubectl get -o yaml` hides it and
    /// k9s's `m` toggles it).
    pub managed_fields: bool,
    /// Show Secret values (masked by default). Revealed text holds the values as the API serves
    /// them (base64 under `data`, plaintext with [`decode_secrets`](Self::decode_secrets)) and
    /// lives in memory only.
    pub reveal_secrets: bool,
    /// Show revealed Secret values decoded, as text (E12-S11). Has no effect while the values
    /// are masked: hiding them again masks them, and revealing returns to the decoded text.
    pub decode_secrets: bool,
}

/// What was left out of the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HiddenKind {
    /// `metadata.managedFields` (`count` entries).
    ManagedFields,
    /// The `kubectl.kubernetes.io/last-applied-configuration` annotation of a Secret, which
    /// carries a copy of its data.
    LastApplied,
}

/// A part of the object the text does not show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HiddenRegion {
    /// What is hidden.
    pub kind: HiddenKind,
    /// Byte offset in the text of the `metadata` key it was removed from.
    pub at: usize,
    /// How many entries were removed (`managedFields` entries; 1 for an annotation).
    pub count: usize,
}

/// A Secret value shown as [`MASKED`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaskedRegion {
    /// Bytes of the placeholder in the text.
    pub range: Range<usize>,
    /// The key it stands for (`password`), which is not secret.
    pub key: String,
}

/// An object as editor text, with what was hidden, masked and dimmed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderedManifest {
    /// The YAML text.
    pub text: String,
    /// What the text leaves out.
    pub hidden: Vec<HiddenRegion>,
    /// The placeholders standing for Secret values.
    pub masked: Vec<MaskedRegion>,
    /// Ranges to draw dimmed: each object's `status` (the server owns it).
    pub dimmed: Vec<Range<usize>>,
}

impl RenderedManifest {
    /// How many `managedFields` entries were left out, over all objects.
    pub fn hidden_managed_fields(&self) -> usize {
        self.hidden
            .iter()
            .filter(|region| region.kind == HiddenKind::ManagedFields)
            .map(|region| region.count)
            .sum()
    }
}

/// Why an object could not be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderError(pub String);

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the object cannot be written as YAML: {}", self.0)
    }
}

impl std::error::Error for RenderError {}

/// What [`transform`] left out of one object, before the text exists.
pub(super) struct Left {
    pub(super) kind: HiddenKind,
    pub(super) count: usize,
    /// Which object of the text (0 for a plain object, the item index of a List).
    pub(super) object: usize,
}

/// Renders `resource` as editor text: see [`crate::open`].
///
/// # Errors
///
/// A [`RenderError`] with the serialiser's message (it does not fail for any JSON a cluster
/// serves).
pub fn render_resource(
    resource: &Resource,
    options: RenderOptions,
) -> Result<RenderedManifest, RenderError> {
    let mut value = resource.to_value();
    let left = transform(&mut value, options);
    let value = sorted(value);
    let text = serde_saphyr::to_string(&value).map_err(|error| RenderError(error.to_string()))?;
    Ok(regions::locate(text, &left))
}

/// Removes what `options` hides from `value` (a single object or a List), in place.
pub(super) fn transform(value: &mut Value, options: RenderOptions) -> Vec<Left> {
    let mut left = Vec::new();
    let list = value
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(is_list);
    if list && let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
        for (index, item) in items.iter_mut().enumerate() {
            transform_object(item, options, index, &mut left);
        }
    } else {
        transform_object(value, options, 0, &mut left);
    }
    left
}

pub(super) fn is_list(kind: &str) -> bool {
    kind == "List" || kind.ends_with("List")
}

fn transform_object(
    object: &mut Value,
    options: RenderOptions,
    index: usize,
    left: &mut Vec<Left>,
) {
    let Some(map) = object.as_object_mut() else {
        return;
    };
    if !options.managed_fields
        && let Some(metadata) = map.get_mut("metadata").and_then(Value::as_object_mut)
        && let Some(removed) = metadata.shift_remove("managedFields")
    {
        let count = removed.as_array().map_or(1, Vec::len);
        left.push(Left {
            kind: HiddenKind::ManagedFields,
            count,
            object: index,
        });
    }
    let is_secret = map.get("kind").and_then(Value::as_str) == Some("Secret");
    if is_secret && options.reveal_secrets && options.decode_secrets {
        if secret::to_decoded(object) {
            left.push(Left {
                kind: HiddenKind::LastApplied,
                count: 1,
                object: index,
            });
        }
        return;
    }
    if is_secret && !options.reveal_secrets {
        for field in ["data", "stringData"] {
            if let Some(values) = map.get_mut(field).and_then(Value::as_object_mut) {
                for value in values.values_mut() {
                    *value = Value::String(MASKED.to_owned());
                }
            }
        }
        if let Some(annotations) = map
            .get_mut("metadata")
            .and_then(|metadata| metadata.get_mut("annotations"))
            .and_then(Value::as_object_mut)
            && annotations.shift_remove(secret::LAST_APPLIED).is_some()
        {
            left.push(Left {
                kind: HiddenKind::LastApplied,
                count: 1,
                object: index,
            });
        }
    }
}

/// `value` with the keys of every object in alphabetical order (what kubectl prints).
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect::<Map<String, Value>>(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}
