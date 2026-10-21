//! [`Values`]: the YAML/JSON tree of a release's or chart's values.

use std::fmt;
use std::sync::Arc;

use serde_json::{Map, Value};

use super::error::HelmError;

/// A values tree (`values.yaml`, `--set`, a release's config).
///
/// Cheap to clone (the tree is shared). `Debug` prints only the top-level key count:
/// values can embed credentials and must not reach logs.
#[derive(Clone, PartialEq)]
pub struct Values(Arc<Value>);

impl Default for Values {
    /// No values: an empty map.
    fn default() -> Self {
        Self(Arc::new(Value::Object(Map::new())))
    }
}

impl fmt::Debug for Values {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Values({} top-level keys, redacted)",
            self.keys().count()
        )
    }
}

impl Values {
    /// Wraps a JSON tree. `null` becomes the empty map (an empty `values.yaml`).
    pub fn from_json(value: Value) -> Self {
        match value {
            Value::Null => Self::default(),
            other => Self(Arc::new(other)),
        }
    }

    /// Parses YAML (or JSON, which is YAML). Empty text is the empty map.
    ///
    /// # Errors
    ///
    /// [`HelmError::Parse`] when the text is not valid YAML.
    pub fn from_yaml(text: &str) -> Result<Self, HelmError> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_saphyr::from_str::<Value>(text)
            .map(Self::from_json)
            .map_err(|e| HelmError::Parse(format!("values are not valid YAML: {e}")))
    }

    /// The tree as JSON.
    pub fn as_json(&self) -> &Value {
        &self.0
    }

    /// The tree as YAML text. A trailing newline is guaranteed.
    ///
    /// # Errors
    ///
    /// [`HelmError::Parse`] if the serializer fails.
    pub fn to_yaml(&self) -> Result<String, HelmError> {
        serde_saphyr::to_string(&*self.0)
            .map_err(|e| HelmError::Parse(format!("values cannot be written as YAML: {e}")))
    }

    /// `true` for the empty map (or an empty list/scalar-free tree).
    pub fn is_empty(&self) -> bool {
        match &*self.0 {
            Value::Object(map) => map.is_empty(),
            Value::Array(items) => items.is_empty(),
            Value::Null => true,
            _ => false,
        }
    }

    /// Top-level keys, in document order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0
            .as_object()
            .into_iter()
            .flat_map(|map| map.keys().map(String::as_str))
    }

    /// The value at a dotted `path` (`image.tag`, `ingress.hosts.0`). Path segments that
    /// are numbers index lists.
    pub fn get(&self, path: &str) -> Option<&Value> {
        let mut current = &*self.0;
        if path.is_empty() {
            return Some(current);
        }
        for segment in path.split('.') {
            current = match current {
                Value::Object(map) => map.get(segment)?,
                Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        Some(current)
    }

    /// `base` overlaid with `overlay` the way Helm coalesces values: maps merge key by key,
    /// every other value (lists included) is replaced, and a `null` in the overlay deletes
    /// the key.
    pub fn merge(base: &Values, overlay: &Values) -> Values {
        let mut merged = (*base.0).clone();
        merge_into(&mut merged, &overlay.0);
        Values(Arc::new(merged))
    }
}

fn merge_into(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                if value.is_null() {
                    base_map.remove(key);
                } else if let Some(existing) = base_map.get_mut(key) {
                    merge_into(existing, value);
                } else {
                    base_map.insert(key.clone(), strip_nulls(value));
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

/// Nulls inside a map that is inserted whole are deletions too: Helm drops them.
fn strip_nulls(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), strip_nulls(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}
