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
use futures::StreamExt as _;
use futures::stream::BoxStream;
use oxikube_domain::OxiResult;

/// The chunks of a [`FsPort::write_stream`]: each is the next piece of the file, and an `Err`
/// abandons the write.
pub type FileChunks = BoxStream<'static, OxiResult<Vec<u8>>>;

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
///
/// # Effects
///
/// Mutating on the local filesystem only ([`write`](Self::write),
/// [`write_private`](Self::write_private), [`remove`](Self::remove)); never a cluster mutation.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`NotFound`](oxikube_domain::ErrorKind::NotFound) for a missing path,
/// [`Forbidden`](oxikube_domain::ErrorKind::Forbidden) for a permission failure,
/// [`Validation`](oxikube_domain::ErrorKind::Validation) for a path of the wrong kind (a file
/// where a directory is expected), [`Internal`](oxikube_domain::ErrorKind::Internal) for other
/// I/O failures.
#[async_trait]
pub trait FsPort: Send + Sync {
    /// The whole content of the file at `path`. `NotFound` when absent.
    async fn read(&self, path: &Path) -> OxiResult<Vec<u8>>;

    /// Writes `contents` to `path`, creating parent directories and replacing any
    /// existing file atomically (temp file + rename).
    async fn write(&self, path: &Path, contents: &[u8]) -> OxiResult<()>;

    /// Writes the chunks `chunks` yields, in order, as one file: [`write`](Self::write)'s
    /// atomic replace, without holding the whole content in memory (a log export of a million
    /// lines). Nothing appears at `path` until the stream ended without an error; an `Err`
    /// chunk, or dropping this future, leaves the old file (if any) as it was.
    ///
    /// The default collects the chunks and calls `write`, which is right for an in-memory fake;
    /// a real filesystem overrides it to stream into the temp file.
    async fn write_stream(&self, path: &Path, mut chunks: FileChunks) -> OxiResult<()> {
        let mut contents = Vec::new();
        while let Some(chunk) = chunks.next().await {
            contents.extend_from_slice(&chunk?);
        }
        self.write(path, &contents).await
    }

    /// Like [`write`](Self::write) for a file that may hold credentials (a pasted kubeconfig):
    /// the file is readable and writable by its owner only (mode `0600` on unix; created
    /// parent directories `0700`), from the moment it exists. Other platforms rely on the
    /// user profile's own access rules.
    async fn write_private(&self, path: &Path, contents: &[u8]) -> OxiResult<()>;

    /// Deletes the file at `path` (not a directory). Returns whether there was one: removing a
    /// path that is already gone is `Ok(false)`, not an error.
    async fn remove(&self, path: &Path) -> OxiResult<bool>;

    /// The entries directly inside the directory `path`, ordered by path.
    async fn list(&self, path: &Path) -> OxiResult<Vec<DirEntry>>;

    /// Changes under `path` (recursively for a directory) from now on. Dropping the
    /// stream stops the watch.
    fn watch(&self, path: &Path) -> BoxStream<'static, FsEvent>;
}
