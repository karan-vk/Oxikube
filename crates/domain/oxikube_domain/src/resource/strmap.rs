//! [`StrMap`]: the string maps of `metadata` (labels, annotations) as one shared sorted slice.
//!
//! A `BTreeMap` allocates a 380-byte node for its first entry, and every pod owns two of them.
//! Pods of one ReplicaSet have the same labels, so [`StrMap`] holds its entries in an
//! `Arc<[(key, value)]>` sorted by key: a lookup is a binary search, a clone is a reference
//! count, and equal maps built through [`StrMap::shared`] are one allocation (E07-P603). An empty
//! map allocates nothing.

use std::fmt;
use std::sync::Arc;

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, SerializeMap, Serializer};

use crate::intern::intern_pairs;

type Pair = (Arc<str>, Arc<str>);

/// A string-to-string map with ordered keys, cheap to clone. See the [module docs](self).
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct StrMap {
    entries: Option<Arc<[Pair]>>,
}

impl StrMap {
    /// The empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// The map of `pairs` (a repeated key keeps its last value), sharing storage with every equal
    /// map built the same way.
    pub fn shared(pairs: impl IntoIterator<Item = Pair>) -> Self {
        let mut pairs: Vec<Pair> = pairs.into_iter().collect();
        sort_unique(&mut pairs);
        Self::from_sorted(pairs, true)
    }

    fn from_sorted(pairs: Vec<Pair>, share: bool) -> Self {
        if pairs.is_empty() {
            return Self::default();
        }
        Self {
            entries: Some(if share {
                intern_pairs(pairs)
            } else {
                Arc::from(pairs)
            }),
        }
    }

    fn slice(&self) -> &[Pair] {
        self.entries.as_deref().unwrap_or(&[])
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.slice().len()
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_none()
    }

    /// The value of `key`.
    pub fn get(&self, key: &str) -> Option<&Arc<str>> {
        let entries = self.slice();
        entries
            .binary_search_by(|(k, _)| (**k).cmp(key))
            .ok()
            .map(|at| &entries[at].1)
    }

    /// Whether `key` is present.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// The entries in key order.
    pub fn iter(&self) -> StrMapIter<'_> {
        StrMapIter(self.slice().iter())
    }

    /// The keys in order.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &Arc<str>> + Clone {
        self.slice().iter().map(|(k, _)| k)
    }

    /// The values in key order.
    pub fn values(&self) -> impl ExactSizeIterator<Item = &Arc<str>> + Clone {
        self.slice().iter().map(|(_, v)| v)
    }

    /// Sets `key` to `value`, returning the value it replaced. Rebuilds the entries (the old ones
    /// stay with any clone), so use it for building and tests, not on a hot path.
    pub fn insert(&mut self, key: Arc<str>, value: Arc<str>) -> Option<Arc<str>> {
        let mut pairs = self.slice().to_vec();
        let old = match pairs.binary_search_by(|(k, _)| k.cmp(&key)) {
            Ok(at) => Some(std::mem::replace(&mut pairs[at].1, value)),
            Err(at) => {
                pairs.insert(at, (key, value));
                None
            }
        };
        *self = Self::from_sorted(pairs, false);
        old
    }

    /// Keeps the entries for which `keep` is true.
    pub fn retain(&mut self, mut keep: impl FnMut(&Arc<str>, &Arc<str>) -> bool) {
        let kept: Vec<Pair> = self
            .slice()
            .iter()
            .filter(|(k, v)| keep(k, v))
            .cloned()
            .collect();
        if kept.len() != self.len() {
            *self = Self::from_sorted(kept, false);
        }
    }
}

/// Sorts by key and keeps the last of any repeated key (as inserting one by one would).
fn sort_unique(pairs: &mut Vec<Pair>) {
    pairs.reverse();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs.dedup_by(|later, earlier| later.0 == earlier.0);
}

impl FromIterator<Pair> for StrMap {
    fn from_iter<I: IntoIterator<Item = Pair>>(iter: I) -> Self {
        let mut pairs: Vec<Pair> = iter.into_iter().collect();
        sort_unique(&mut pairs);
        Self::from_sorted(pairs, false)
    }
}

impl<'a> IntoIterator for &'a StrMap {
    type Item = (&'a Arc<str>, &'a Arc<str>);
    type IntoIter = StrMapIter<'a>;

    fn into_iter(self) -> StrMapIter<'a> {
        self.iter()
    }
}

/// Iterator over the entries of a [`StrMap`], in key order.
#[derive(Clone)]
pub struct StrMapIter<'a>(std::slice::Iter<'a, Pair>);

impl<'a> Iterator for StrMapIter<'a> {
    type Item = (&'a Arc<str>, &'a Arc<str>);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (k, v))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for StrMapIter<'_> {}

impl fmt::Debug for StrMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl Serialize for StrMap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (key, value) in self.iter() {
            map.serialize_entry(&**key, &**value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for StrMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = std::collections::BTreeMap::<Arc<str>, Arc<str>>::deserialize(deserializer)?;
        Ok(map.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(k: &str, v: &str) -> Pair {
        (Arc::from(k), Arc::from(v))
    }

    #[test]
    fn lookups_iterate_in_key_order() {
        let map: StrMap = [pair("b", "2"), pair("a", "1"), pair("c", "3")]
            .into_iter()
            .collect();
        assert_eq!(map.len(), 3);
        assert_eq!(map.get("b").map(|v| &**v), Some("2"));
        assert!(map.get("z").is_none());
        let keys: Vec<&str> = map.keys().map(|k| &**k).collect();
        assert_eq!(keys, ["a", "b", "c"]);
        let seen: Vec<_> = (&map).into_iter().map(|(k, _)| &**k).collect();
        assert_eq!(seen, ["a", "b", "c"]);
    }

    #[test]
    fn a_repeated_key_keeps_the_last_value() {
        let map: StrMap = [pair("a", "1"), pair("a", "2")].into_iter().collect();
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("a").map(|v| &**v), Some("2"));
    }

    #[test]
    fn equal_shared_maps_are_one_allocation() {
        let a = StrMap::shared([pair("app", "web"), pair("tier", "fe")]);
        let b = StrMap::shared([pair("tier", "fe"), pair("app", "web")]);
        assert_eq!(a, b);
        let (Some(x), Some(y)) = (&a.entries, &b.entries) else {
            panic!("not empty");
        };
        assert!(Arc::ptr_eq(x, y));
    }

    #[test]
    fn insert_and_retain_do_not_touch_clones() {
        let mut map = StrMap::shared([pair("a", "1")]);
        let before = map.clone();
        assert_eq!(map.insert("b".into(), "2".into()), None);
        assert_eq!(map.insert("a".into(), "9".into()).as_deref(), Some("1"));
        map.retain(|k, _| &**k != "b");
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("a").map(|v| &**v), Some("9"));
        assert_eq!(before.get("a").map(|v| &**v), Some("1"));
        assert!(StrMap::new().is_empty());
    }

    #[test]
    fn serialises_as_a_json_object() {
        let map = StrMap::shared([pair("b", "2"), pair("a", "1")]);
        let text = serde_json::to_string(&map).unwrap();
        assert_eq!(text, r#"{"a":"1","b":"2"}"#);
        assert_eq!(serde_json::from_str::<StrMap>(&text).unwrap(), map);
    }
}
