//! The `/openapi/v3` index: which group-version document lives at which URL.
//!
//! Pure parsing, no I/O: the index document maps group-version keys to entries
//! carrying a `serverRelativeURL` with a `?hash=` query (the content hash of
//! the group document). The hash is what validates the disk cache.

use std::collections::BTreeMap;

/// One group-version document of the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexEntry {
    /// The document URL as the server gave it (`/openapi/v3/apis/apps/v1?hash=...`).
    pub(crate) url: String,
    /// The `hash` query value, empty when the server sent none.
    pub(crate) hash: String,
}

/// The parsed index: group-version key (`api/v1`, `apis/apps/v1`) → entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Index {
    pub(crate) entries: BTreeMap<String, IndexEntry>,
}

impl Index {
    /// The index key holding `gvk`'s group-version document.
    pub(crate) fn key_for(group: &str, version: &str) -> String {
        if group.is_empty() {
            format!("api/{version}")
        } else {
            format!("apis/{group}/{version}")
        }
    }

    /// The entry for `gvk`'s group-version, if the server listed one.
    pub(crate) fn entry_for(&self, group: &str, version: &str) -> Option<&IndexEntry> {
        self.entries.get(&Self::key_for(group, version))
    }
}

/// Parses an `/openapi/v3` index document. Unknown shapes (a missing `paths`
/// map, entries without a URL) are skipped, never an error: a document with
/// no usable entry simply resolves nothing.
pub(crate) fn parse_index(document: &serde_json::Value) -> Index {
    let mut index = Index::default();
    let Some(paths) = document.get("paths").and_then(serde_json::Value::as_object) else {
        return index;
    };
    for (key, value) in paths {
        let url = value
            .get("serverRelativeURL")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(key)
            .to_owned();
        if url.is_empty() {
            continue;
        }
        index.entries.insert(
            key.trim_start_matches('/').to_owned(),
            IndexEntry {
                hash: hash_of(&url),
                url,
            },
        );
    }
    index
}

/// The `hash` query value of a `serverRelativeURL`, or empty.
fn hash_of(url: &str) -> String {
    url.split('?')
        .skip(1)
        .flat_map(|query| query.split('&'))
        .find_map(|pair| pair.strip_prefix("hash="))
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn keys_distinguish_core_from_groups() {
        assert_eq!(Index::key_for("", "v1"), "api/v1");
        assert_eq!(Index::key_for("apps", "v1"), "apis/apps/v1");
    }

    #[test]
    fn entries_carry_url_and_hash() {
        let index = parse_index(&json!({
            "paths": {
                "api/v1": {"serverRelativeURL": "/openapi/v3/api/v1?hash=AAA="},
                "apis/apps/v1": {"serverRelativeURL": "/openapi/v3/apis/apps/v1?hash=BBB="},
            }
        }));
        let core = index.entry_for("", "v1").expect("core");
        assert_eq!(core.url, "/openapi/v3/api/v1?hash=AAA=");
        assert_eq!(core.hash, "AAA=");
        assert_eq!(index.entry_for("apps", "v1").expect("apps").hash, "BBB=");
        assert!(index.entry_for("batch", "v1").is_none());
    }

    #[test]
    fn missing_shapes_resolve_nothing() {
        assert!(parse_index(&json!({})).entries.is_empty());
        assert!(parse_index(&json!({"paths": []})).entries.is_empty());
        let index = parse_index(&json!({"paths": {"api/v1": {}}}));
        assert_eq!(
            index.entry_for("", "v1").expect("key fallback").url,
            "api/v1"
        );
    }
}
