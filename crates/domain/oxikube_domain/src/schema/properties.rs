//! [`Properties`]: the named children of an object schema, stored compactly.

use std::collections::BTreeMap;

use super::JsonSchema;

/// Named child schemas of an object, sorted by name.
///
/// A flattened Kubernetes schema is a tree of a thousand or more nodes, most
/// of them objects with a handful of properties. A `BTreeMap` allocates a full
/// eleven-slot node per object however few entries it holds, which was most of
/// the heap of a cached schema; a sorted boxed slice costs exactly its entries.
/// Lookup is a binary search.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Properties(Box<[(String, JsonSchema)]>);

impl Properties {
    /// The schema of property `name`, if the object declares it.
    pub fn get(&self, name: &str) -> Option<&JsonSchema> {
        self.0
            .binary_search_by(|(key, _)| key.as_str().cmp(name))
            .ok()
            .map(|at| &self.0[at].1)
    }

    /// Whether the object declares property `name`.
    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The declared properties in name order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &JsonSchema)> {
        self.0.iter().map(|(name, schema)| (name.as_str(), schema))
    }

    /// The declared property names in order.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.iter().map(|(name, _)| name)
    }

    /// The declared property schemas in name order.
    pub fn values(&self) -> impl ExactSizeIterator<Item = &JsonSchema> {
        self.0.iter().map(|(_, schema)| schema)
    }

    /// How many properties are declared.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no property is declared.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Adds the properties of `other`; on a name clash `other` wins.
    pub(super) fn merge(&mut self, other: Properties) {
        if other.is_empty() {
            return;
        }
        let mut map: BTreeMap<String, JsonSchema> =
            std::mem::take(&mut self.0).into_vec().into_iter().collect();
        map.extend(other.0.into_vec());
        *self = map.into_iter().collect();
    }
}

impl FromIterator<(String, JsonSchema)> for Properties {
    /// Sorts by name; a repeated name keeps its last schema.
    fn from_iter<I: IntoIterator<Item = (String, JsonSchema)>>(iter: I) -> Self {
        let map: BTreeMap<String, JsonSchema> = iter.into_iter().collect();
        Self(map.into_iter().collect::<Vec<_>>().into_boxed_slice())
    }
}
