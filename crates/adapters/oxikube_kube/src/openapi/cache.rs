//! The disk cache: raw group-version documents keyed by cluster and index hash.
//!
//! One file per group-version under `<cache_dir>/<cluster>/<file>`, holding
//! `{"hash": <index hash>, "document": <raw group document>}`. A read whose
//! hash no longer matches the index is a miss (the file is replaced on the
//! next fetch), so a server upgrade never serves stale schemas. Writes are
//! best-effort: a cache failure warns and the call still serves from memory.
//!
//! The cache holds public API schemas only, never object data or credentials
//! (non-negotiable 5).

use std::path::{Path, PathBuf};

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::FsPort;

/// A cached group-version document with the index hash it was stored under.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CachedDocument {
    /// The index entry hash of the stored document.
    pub(crate) hash: String,
    /// The raw group-version document.
    pub(crate) document: serde_json::Value,
}

/// The file holding `group_version` (`apps/v1`, or `v1` for the core group) of
/// `cluster`: slashes become underscores so the key is one path segment.
pub(crate) fn path_for(cache_dir: &Path, cluster: &ClusterId, group_version: &str) -> PathBuf {
    cache_dir
        .join(cluster.as_str())
        .join(format!("{}.json", group_version.replace('/', "_")))
}

/// Reads the cached document for `group_version` whose hash is `expected_hash`.
/// `Ok(None)` on a miss: no file, an unreadable file, a wrong hash, or a body
/// that is not a JSON object. Only the hash is compared; the body is never
/// logged.
pub(crate) async fn read(
    fs: &dyn FsPort,
    cache_dir: &Path,
    cluster: &ClusterId,
    group_version: &str,
    expected_hash: &str,
) -> OxiResult<Option<CachedDocument>> {
    let bytes = match fs.read(&path_for(cache_dir, cluster, group_version)).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == oxikube_domain::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Ok(None),
    };
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    let hash = stored
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if !expected_hash.is_empty() && hash != expected_hash {
        return Ok(None);
    }
    let Some(document) = stored.get("document").cloned() else {
        return Ok(None);
    };
    if !document.is_object() {
        return Ok(None);
    }
    Ok(Some(CachedDocument {
        hash: hash.to_owned(),
        document,
    }))
}

/// Stores `document` for `group_version` under `hash`. Failures are the
/// caller's to demote to a warning: the cache must never fail a schema lookup.
pub(crate) async fn write(
    fs: &dyn FsPort,
    cache_dir: &Path,
    cluster: &ClusterId,
    group_version: &str,
    hash: &str,
    document: &serde_json::Value,
) -> OxiResult<()> {
    let stored = serde_json::json!({"hash": hash, "document": document});
    let bytes = serde_json::to_vec(&stored)
        .map_err(|e| oxikube_domain::OxiError::internal(format!("openapi cache encode: {e}")))?;
    fs.write(&path_for(cache_dir, cluster, group_version), &bytes)
        .await
}

/// Removes every cached file of `cluster`. Missing files are fine.
pub(crate) async fn remove_cluster(
    fs: &dyn FsPort,
    cache_dir: &Path,
    cluster: &ClusterId,
) -> OxiResult<()> {
    let dir = cache_dir.join(cluster.as_str());
    let entries = match fs.list(&dir).await {
        Ok(entries) => entries,
        Err(err) if err.kind() == oxikube_domain::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let _ = fs.remove(&entry.path).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use oxikube_testkit::FakeFsPort;
    use serde_json::json;

    use super::*;

    fn cluster() -> ClusterId {
        ClusterId::new("cache-test", &"kind-oxikube".into())
    }

    #[tokio::test]
    async fn round_trip_hits_on_matching_hash() {
        let fs = FakeFsPort::new();
        let dir = Path::new("/cache");
        write(
            &fs,
            dir,
            &cluster(),
            "apps/v1",
            "abc=",
            &json!({"components": {}}),
        )
        .await
        .unwrap();
        let hit = read(&fs, dir, &cluster(), "apps/v1", "abc=").await.unwrap();
        assert_eq!(
            hit,
            Some(CachedDocument {
                hash: "abc=".into(),
                document: json!({"components": {}})
            })
        );
    }

    #[tokio::test]
    async fn stale_hash_and_missing_file_miss() {
        let fs = FakeFsPort::new();
        let dir = Path::new("/cache");
        write(
            &fs,
            dir,
            &cluster(),
            "apps/v1",
            "old=",
            &json!({"components": {}}),
        )
        .await
        .unwrap();
        assert_eq!(
            read(&fs, dir, &cluster(), "apps/v1", "new=").await.unwrap(),
            None
        );
        assert_eq!(
            read(&fs, dir, &cluster(), "batch/v1", "new=")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn remove_cluster_clears_only_that_cluster() {
        let fs = FakeFsPort::new();
        let dir = Path::new("/cache");
        let other = ClusterId::new("other", &"kind-x".into());
        write(&fs, dir, &cluster(), "apps/v1", "h=", &json!({}))
            .await
            .unwrap();
        write(&fs, dir, &other, "apps/v1", "h=", &json!({}))
            .await
            .unwrap();
        remove_cluster(&fs, dir, &cluster()).await.unwrap();
        assert_eq!(
            read(&fs, dir, &cluster(), "apps/v1", "h=").await.unwrap(),
            None
        );
        assert!(
            read(&fs, dir, &other, "apps/v1", "h=")
                .await
                .unwrap()
                .is_some()
        );
    }
}
