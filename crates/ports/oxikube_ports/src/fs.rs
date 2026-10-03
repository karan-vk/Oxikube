//! [`FsPort`]: small path-based filesystem access, so editors and settings can be faked.
//!
//! # Adapter
//!
//! Implemented by `oxikube_runtime` (the platform layer, on `std::fs` and a file
//! watcher) and faked in `oxikube_testkit` with an in-memory tree. Tests must not
//! start OS watcher threads (`references/testing.md`), so code takes `Arc<dyn FsPort>`.
//!
//! This is local file access for the app's own files and user-chosen paths. It is
//! not a cluster mutation and does not go through `MutationGuard`.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use futures::stream::BoxStream;
use oxikube_domain::OxiResult;

/// The type of a directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link (not followed).
    Symlink,
}

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Full path of the entry.
    pub path: PathBuf,
    /// What it is.
    pub kind: EntryKind,
    /// Size in bytes, for files.
    pub size: Option<u64>,
}

/// What happened to a watched path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FsEventKind {
    /// Created.
    Created,
    /// Content or metadata changed.
    Modified,
    /// Removed.
    Removed,
}

/// A change under a watched path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsEvent {
    /// The path that changed.
    pub path: PathBuf,
    /// What happened.
    pub kind: FsEventKind,
}

/// Filesystem access by path.
#[async_trait]
pub trait FsPort: Send + Sync {
    /// The whole content of the file at `path`. `NotFound` when absent.
    async fn read(&self, path: &Path) -> OxiResult<Vec<u8>>;

    /// Writes `contents` to `path`, creating parent directories and replacing any
    /// existing file atomically (temp file + rename).
    async fn write(&self, path: &Path, contents: &[u8]) -> OxiResult<()>;

    /// The entries directly inside the directory `path`, ordered by path.
    async fn list(&self, path: &Path) -> OxiResult<Vec<DirEntry>>;

    /// Changes under `path` (recursively for a directory) from now on. Dropping the
    /// stream stops the watch.
    fn watch(&self, path: &Path) -> BoxStream<'static, FsEvent>;
}
