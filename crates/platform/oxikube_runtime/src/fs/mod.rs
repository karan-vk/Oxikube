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
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use futures::stream::BoxStream;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{DirEntry, EntryKind, FsEvent, FsPort};

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

/// Replaces `path` with `contents` atomically; `private` creates the file `0600` on unix.
fn write_atomically(path: &Path, contents: &[u8], private: bool) -> OxiResult<()> {
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
    let temp = dir.join(format!(
        ".{}.tmp-{}-{}",
        name.to_string_lossy(),
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let written = options
        .open(&temp)
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
