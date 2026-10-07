//! [`StdFs`]: the [`FsPort`] on `std::fs` and `notify` (E06-S05).
//!
//! The one place the app touches local files by path: the kubeconfig sources screen writes a
//! pasted kubeconfig and deletes it again through it, so the app layer stays plain async Rust
//! and tests use `FakeFsPort`.
//!
//! | Method | What it does |
//! |---|---|
//! | `read` | the whole file; `NotFound` when absent |
//! | `write` | temp file next to the target, `fsync`, rename over it: readers see the old or the new file, never half |
//! | `write_stream` | the same, fed chunk by chunk from a stream (a log export): the file never sits in memory, and an error or a dropped call removes the temp file |
//! | `write_private` | the same, with the file created `0600` (and new parent directories `0700`) on unix, so the content is never readable by others, not even for a moment |
//! | `remove` | deletes a file; a path that is already gone is `Ok(false)` |
//! | `list` | direct children, ordered by path, symlinks reported as such |
//! | `watch` | `notify` on the path; dropping the stream stops the watch |
//!
//! Every method runs its `std::fs` work on tokio's blocking pool, so none of them blocks the
//! caller's executor: call them from a task started with
//! [`spawn_kube`](crate::spawn_kube), never from the UI thread. Error messages name paths and
//! never contents.

mod watch;

use std::fs;
use std::io::{ErrorKind as IoErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::stream::BoxStream;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{DirEntry, EntryKind, FileChunks, FsEvent, FsPort};

/// The [`FsPort`] on the real filesystem. Stateless and free to copy.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdFs;

impl StdFs {
    /// A filesystem port.
    pub fn new() -> Self {
        Self
    }
}

/// Runs `work` on the blocking pool.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> OxiResult<T> + Send + 'static,
) -> OxiResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| OxiError::internal("a file task failed").with_source(err))?
}

/// Maps an I/O failure on `path`, naming the path and the OS error kind only.
fn map_io(action: &str, path: &Path, error: std::io::Error) -> OxiError {
    let shown = path.display();
    match error.kind() {
        IoErrorKind::NotFound => OxiError::not_found(format!("{shown} was not found")),
        IoErrorKind::PermissionDenied => {
            OxiError::forbidden(format!("no permission to {action} {shown}"))
        }
        IoErrorKind::IsADirectory | IoErrorKind::NotADirectory => {
            OxiError::validation(format!("{shown} is not the kind of path expected"))
        }
        kind => {
            OxiError::internal(format!("could not {action} {shown} ({kind})")).with_source(error)
        }
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Creates `dir` and its missing parents; with `private`, the ones it creates are `0700`.
fn create_parents(dir: &Path, private: bool) -> OxiResult<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    #[cfg(not(unix))]
    let _ = private;
    builder
        .create(dir)
        .map_err(|error| map_io("create", dir, error))
}

/// The temp file next to `path` that a write fills before it renames it over `path`: validates
/// the path and creates the parent directories.
fn temp_beside(path: &Path, private: bool) -> OxiResult<PathBuf> {
    let Some(name) = path.file_name() else {
        return Err(OxiError::validation(format!(
            "{} is not a file path",
            path.display()
        )));
    };
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    create_parents(dir, private)?;
    Ok(dir.join(format!(
        ".{}.tmp-{}-{}",
        name.to_string_lossy(),
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    )))
}

/// Creates `temp` (new, `0600` when `private` on unix).
fn create_temp(temp: &Path, private: bool) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    options.open(temp)
}

/// Replaces `path` with `contents` atomically; `private` creates the file `0600` on unix.
fn write_atomically(path: &Path, contents: &[u8], private: bool) -> OxiResult<()> {
    let temp = temp_beside(path, private)?;
    let written = create_temp(&temp, private)
        .and_then(|mut file| {
            file.write_all(contents)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temp, path));
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(map_io("write", path, error));
    }
    Ok(())
}

/// [`write_atomically`] for chunks pulled from `chunks` on this (blocking-pool) thread: `runtime`
/// drives the stream, and `cancelled` (set when the caller dropped the write) is looked at between
/// chunks. Any failure removes the temp file; the old file stays.
fn write_chunks_atomically(
    path: &Path,
    mut chunks: FileChunks,
    runtime: &tokio::runtime::Handle,
    cancelled: &AtomicBool,
) -> OxiResult<()> {
    let temp = temp_beside(path, false)?;
    let mut file = create_temp(&temp, false).map_err(|error| map_io("write", path, error))?;
    let result = (|| -> OxiResult<()> {
        while let Some(chunk) = runtime.block_on(chunks.next()) {
            if cancelled.load(Ordering::Acquire) {
                return Err(OxiError::internal(format!(
                    "writing {} was cancelled",
                    path.display()
                )));
            }
            file.write_all(&chunk?)
                .map_err(|error| map_io("write", path, error))?;
        }
        file.sync_all()
            .map_err(|error| map_io("write", path, error))
    })();
    drop(file);
    let result =
        result.and_then(|()| fs::rename(&temp, path).map_err(|e| map_io("write", path, e)));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Sets its flag when dropped: the blocking thread of a `write_stream` whose future was dropped
/// notices at its next chunk.
struct CancelOnDrop(std::sync::Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[async_trait]
impl FsPort for StdFs {
    async fn read(&self, path: &Path) -> OxiResult<Vec<u8>> {
        let path = path.to_owned();
        blocking(move || fs::read(&path).map_err(|error| map_io("read", &path, error))).await
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> OxiResult<()> {
        let (path, contents) = (path.to_owned(), contents.to_vec());
        blocking(move || write_atomically(&path, &contents, false)).await
    }

    async fn write_stream(&self, path: &Path, chunks: FileChunks) -> OxiResult<()> {
        let path = path.to_owned();
        let runtime = tokio::runtime::Handle::current();
        let cancelled = std::sync::Arc::new(AtomicBool::new(false));
        let _on_drop = CancelOnDrop(cancelled.clone());
        blocking(move || write_chunks_atomically(&path, chunks, &runtime, &cancelled)).await
    }

    async fn write_private(&self, path: &Path, contents: &[u8]) -> OxiResult<()> {
        let (path, contents) = (path.to_owned(), contents.to_vec());
        blocking(move || write_atomically(&path, &contents, true)).await
    }

    async fn remove(&self, path: &Path) -> OxiResult<bool> {
        let path = path.to_owned();
        blocking(move || match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == IoErrorKind::NotFound => Ok(false),
            Err(error) => Err(map_io("remove", &path, error)),
        })
        .await
    }

    async fn list(&self, path: &Path) -> OxiResult<Vec<DirEntry>> {
        let path = path.to_owned();
        blocking(move || {
            let mut entries = Vec::new();
            for entry in fs::read_dir(&path).map_err(|error| map_io("list", &path, error))? {
                let entry = entry.map_err(|error| map_io("list", &path, error))?;
                let meta = entry
                    .metadata()
                    .map_err(|error| map_io("inspect", &entry.path(), error))?;
                let kind = if meta.file_type().is_symlink() {
                    EntryKind::Symlink
                } else if meta.is_dir() {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                };
                entries.push(DirEntry {
                    path: entry.path(),
                    kind,
                    size: (kind == EntryKind::File).then(|| meta.len()),
                });
            }
            entries.sort_by(|a, b| a.path.cmp(&b.path));
            Ok(entries)
        })
        .await
    }

    fn watch(&self, path: &Path) -> BoxStream<'static, FsEvent> {
        watch::watch(PathBuf::from(path))
    }
}

#[cfg(test)]
mod tests;
