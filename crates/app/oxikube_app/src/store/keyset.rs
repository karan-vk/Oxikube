//! [`KeySet`]: the keys behind one entry of a cache index, with no allocation for the common
//! single key.
//!
//! The cache indexes objects by name, namespace and label. Names are nearly unique, and most
//! `pod-template-hash` values belong to a handful of pods, so most index entries hold one key. A
//! `HashSet` costs a heap table of 100 to 150 bytes for that one key; this holds it inline
//! (E07-P603) and turns into a set only when a second key arrives.

use std::collections::HashSet;

use super::object::ObjectKey;

/// A set of [`ObjectKey`]s. See the [module docs](self).
#[derive(Debug, Default)]
pub(crate) enum KeySet {
    #[default]
    Empty,
    One(ObjectKey),
    Many(HashSet<ObjectKey>),
}

impl KeySet {
    /// Adds `key`.
    pub fn insert(&mut self, key: ObjectKey) {
        match self {
            KeySet::Empty => *self = KeySet::One(key),
            KeySet::One(only) if *only == key => {}
            KeySet::One(_) => {
                let KeySet::One(first) = std::mem::take(self) else {
                    return;
                };
                *self = KeySet::Many(HashSet::from([first, key]));
            }
            KeySet::Many(set) => {
                set.insert(key);
            }
        }
    }

    /// Removes `key`.
    pub fn remove(&mut self, key: &ObjectKey) {
        match self {
            KeySet::One(only) if only == key => *self = KeySet::Empty,
            KeySet::Many(set) => {
                set.remove(key);
                if set.len() == 1
                    && let Some(rest) = set.iter().next().cloned()
                {
                    *self = KeySet::One(rest);
                }
            }
            KeySet::Empty | KeySet::One(_) => {}
        }
    }

    /// Whether the set has no keys.
    pub fn is_empty(&self) -> bool {
        matches!(self, KeySet::Empty)
    }

    /// The keys, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = &ObjectKey> {
        let (one, many) = match self {
            KeySet::Empty => (None, None),
            KeySet::One(key) => (Some(key), None),
            KeySet::Many(set) => (None, Some(set.iter())),
        };
        one.into_iter().chain(many.into_iter().flatten())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> ObjectKey {
        ObjectKey::new(Some("ns"), name)
    }

    #[test]
    fn grows_from_one_key_to_a_set_and_back() {
        let mut set = KeySet::default();
        assert!(set.is_empty());
        set.insert(key("a"));
        set.insert(key("a"));
        assert_eq!(
            (set.iter().count(), matches!(set, KeySet::One(_))),
            (1, true)
        );
        set.insert(key("b"));
        set.insert(key("c"));
        assert_eq!(set.iter().count(), 3);
        set.remove(&key("a"));
        set.remove(&key("zzz"));
        assert_eq!(set.iter().count(), 2);
        set.remove(&key("b"));
        assert!(matches!(&set, KeySet::One(k) if *k == key("c")));
        set.remove(&key("c"));
        assert!(set.is_empty());
        assert_eq!(set.iter().count(), 0);
    }

    #[test]
    fn iterates_every_key_once() {
        let mut set = KeySet::default();
        for name in ["a", "b", "c", "d"] {
            set.insert(key(name));
        }
        let mut names: Vec<String> = set.iter().map(|k| k.name.to_string()).collect();
        names.sort();
        assert_eq!(names, ["a", "b", "c", "d"]);
    }
}
