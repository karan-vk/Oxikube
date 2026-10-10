//! The disk cache: raw group-version documents keyed by cluster, server
//! version and index hash.
//!
//! One file per group-version at
//! `<cache_dir>/<cluster>/<server version>/<group-version>-<hash>.json`, holding
//! the document exactly as the server sent it. The index entry's `hash` is the
//! server's content hash of that document, so a file is served only when its
//! name carries the hash the fresh index lists: a CRD change or a server
//! upgrade produces a different name and the old file is a miss (and is removed
//! on the next write). Without a hash there is nothing to validate against, so
//! nothing is read or written. Writes are best-effort: a cache failure warns
//! and the call still serves from memory.
//!
//! The cache holds public API schemas only, never object data or credentials
//! (non-negotiable 5).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{EntryKind, FsPort};

/// The server version used as the cache key when `/version` could not be read.
pub(super) const UNKNOWN_VERSION: &str = "unknown";

/// Where the raw documents of one cluster live. See the module docs.
pub(super) struct DiskCache {
    fs: Arc<dyn FsPort>,
    dir: PathBuf,
    /// Set once the other server versions' files were pruned.
    pruned_versions: AtomicBool,
}

/// One document's key: its server version, group-version and index hash.
pub(super) struct Key<'a> {
    pub(super) server_version: &'a str,
    pub(super) group_version: &'a str,
    pub(super) hash: &'a str,
}

impl DiskCache {
    pub(super) fn new(fs: Arc<dyn FsPort>, dir: PathBuf) -> Self {
        Self {
            fs,
            dir,
            pruned_versions: AtomicBool::new(false),
        }
    }

    /// The cached document bytes for `key`, or `None` on any miss (no hash, no
    /// file, an unreadable file). The body is never logged.
    pub(super) async fn read(&self, cluster: &ClusterId, key: &Key<'_>) -> Option<Vec<u8>> {
        if key.hash.is_empty() {
            return None;
        }
        self.fs.read(&self.file(cluster, key)).await.ok()
    }

    /// Stores `document` under `key`, then drops the group-version's files of
    /// older hashes and (once per instance) every other server version's files.
    /// A key without a hash is not stored.
    pub(super) async fn write(
        &self,
        cluster: &ClusterId,
        key: &Key<'_>,
        document: &[u8],
    ) -> OxiResult<()> {
        if key.hash.is_empty() {
            return Ok(());
        }
        let file = self.file(cluster, key);
        self.fs.write(&file, document).await?;
        self.prune(cluster, key, &file).await;
        Ok(())
    }

    fn version_dir(&self, cluster: &ClusterId, server_version: &str) -> PathBuf {
        self.dir
            .join(cluster.as_str())
            .join(segment(server_version))
    }

    fn file(&self, cluster: &ClusterId, key: &Key<'_>) -> PathBuf {
        self.version_dir(cluster, key.server_version).join(format!(
            "{}-{}.json",
            segment(key.group_version),
            segment(key.hash)
        ))
    }

    /// Best-effort cleanup; failures only leave a stale file behind.
    async fn prune(&self, cluster: &ClusterId, key: &Key<'_>, keep: &std::path::Path) {
        let prefix = format!("{}-", segment(key.group_version));
        let dir = self.version_dir(cluster, key.server_version);
        for entry in self.fs.list(&dir).await.unwrap_or_default() {
            let stale = entry.path != keep
                && entry
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix));
            if stale {
                let _ = self.fs.remove(&entry.path).await;
            }
        }
        // Under the fallback key the real version is unknown, so its files are not stale.
        if key.server_version == UNKNOWN_VERSION
            || self.pruned_versions.swap(true, Ordering::Relaxed)
        {
            return;
        }
        let current = segment(key.server_version);
        for sibling in self
            .fs
            .list(&self.dir.join(cluster.as_str()))
            .await
            .unwrap_or_default()
        {
            let other = sibling.kind == EntryKind::Dir
                && sibling.path.file_name().and_then(|n| n.to_str()) != Some(current.as_str());
            if other {
                for file in self.fs.list(&sibling.path).await.unwrap_or_default() {
                    let _ = self.fs.remove(&file.path).await;
                }
            }
        }
    }
}

/// One safe path segment, injective: bytes outside `[A-Za-z0-9.-]` (and `_`
/// itself) become `_` plus two hex digits, so `a=` and `a/` never share a file
/// (`A/b+c=` is `A_2fb_2bc_3d`).
fn segment(text: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "_{byte:02x}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use oxikube_testkit::FakeFsPort;

    use super::*;

    fn cluster() -> ClusterId {
        ClusterId::new("cache-test", &"kind-oxikube".into())
    }

    fn cache(fs: &Arc<FakeFsPort>) -> DiskCache {
        DiskCache::new(fs.clone(), PathBuf::from("/cache"))
    }

    fn key<'a>(version: &'a str, hash: &'a str) -> Key<'a> {
        Key {
            server_version: version,
            group_version: "apis/apps/v1",
            hash,
        }
    }

    #[tokio::test]
    async fn round_trip_hits_only_for_the_same_version_and_hash() {
        let fs = Arc::new(FakeFsPort::new());
        let cache = cache(&fs);
        cache
            .write(&cluster(), &key("v1.31.0", "a="), b"doc")
            .await
            .unwrap();
        let hit = cache.read(&cluster(), &key("v1.31.0", "a=")).await;
        assert_eq!(hit.as_deref(), Some(&b"doc"[..]));
        assert!(
            cache
                .read(&cluster(), &key("v1.31.0", "b="))
                .await
                .is_none()
        );
        assert!(
            cache
                .read(&cluster(), &key("v1.32.0", "a="))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn no_hash_means_no_cache() {
        let fs = Arc::new(FakeFsPort::new());
        let cache = cache(&fs);
        cache
            .write(&cluster(), &key("v1", ""), b"doc")
            .await
            .unwrap();
        assert!(cache.read(&cluster(), &key("v1", "")).await.is_none());
        assert!(
            fs.list(Path::new("/cache")).await.is_err()
                || fs.list(Path::new("/cache")).await.unwrap().is_empty()
        );
    }

    #[tokio::test]
    async fn a_write_drops_older_hashes_and_other_server_versions() {
        let fs = Arc::new(FakeFsPort::new());
        let first = cache(&fs);
        first
            .write(&cluster(), &key("v1.30.0", "a="), b"old")
            .await
            .unwrap();
        // A new process (fresh prune flag) writes under the upgraded server.
        let second = cache(&fs);
        second
            .write(&cluster(), &key("v1.31.0", "b="), b"new")
            .await
            .unwrap();
        second
            .write(&cluster(), &key("v1.31.0", "c="), b"newer")
            .await
            .unwrap();
        let old_dir = Path::new("/cache").join(cluster().as_str()).join("v1.30.0");
        let new_dir = Path::new("/cache").join(cluster().as_str()).join("v1.31.0");
        // A real filesystem keeps the emptied directory; the fake forgets it.
        assert!(
            fs.list(&old_dir)
                .await
                .map_or(true, |files| files.is_empty())
        );
        let kept = fs.list(&new_dir).await.unwrap();
        assert_eq!(kept.len(), 1, "only the newest hash stays: {kept:?}");
    }

    #[test]
    fn segments_are_one_safe_path_component() {
        assert_eq!(segment("v1.31.0+k3s1"), "v1.31.0_2bk3s1");
        assert_eq!(segment("apis/apps/v1"), "apis_2fapps_2fv1");
        assert_eq!(segment("A/b+c="), "A_2fb_2bc_3d");
        assert_ne!(
            segment("a="),
            segment("a/"),
            "distinct inputs never collide"
        );
        assert_ne!(segment("a_"), segment("a/"), "an underscore is escaped too");
    }
}
