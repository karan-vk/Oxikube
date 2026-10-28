//! The managed kubeconfigs: one file per provider, `<config dir>/kubeconfigs/<cli>.yaml`
//! (`aws.yaml`, `gcloud.yaml`, `az.yaml`), holding only the clusters the user added.
//!
//! Written through [`FsPort::write_private`](oxikube_ports::FsPort::write_private): a temp file
//! next to the target, fsync, rename, owner-only (`0600`), so a crash leaves the old file or the
//! new one, never half of either. The user's own kubeconfig is never read or written here.
//! Pasted kubeconfigs live in the same directory; their names can never be `aws`, `gcloud` or
//! `az` ([`is_managed_stem`](crate::sources::is_managed_stem)), and a file at a managed path
//! that does not start with the [`MARKER`](yaml::MARKER) line is refused, never overwritten.

mod yaml;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{CloudProvider, FsPort};

pub use yaml::ManagedEntry;
pub(crate) use yaml::{MARKER, ManagedKubeconfig};

/// The managed files of one kubeconfigs directory. Cheap to clone.
#[derive(Clone)]
pub(super) struct ManagedFiles {
    fs: Arc<dyn FsPort>,
    dir: PathBuf,
}

impl ManagedFiles {
    pub fn new(fs: Arc<dyn FsPort>, dir: PathBuf) -> Self {
        Self { fs, dir }
    }

    /// `<dir>/<cli>.yaml`.
    pub fn path(&self, provider: CloudProvider) -> PathBuf {
        self.dir.join(format!("{}.yaml", provider.cli()))
    }

    /// The file of `provider`; empty when there is none yet.
    ///
    /// # Errors
    ///
    /// `Conflict` when a file at that path was not written by cloud discovery; `Validation`
    /// when it no longer parses; the filesystem's error.
    pub async fn read(&self, provider: CloudProvider) -> OxiResult<ManagedKubeconfig> {
        let path = self.path(provider);
        let bytes = match self.fs.read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(ManagedKubeconfig::default());
            }
            Err(error) => return Err(error),
        };
        let text = String::from_utf8(bytes).map_err(|_| not_ours(&path))?;
        if !text.starts_with(MARKER) {
            return Err(not_ours(&path));
        }
        ManagedKubeconfig::parse(&text)
    }

    /// Replaces the file of `provider` with `file`, atomically and owner-only.
    ///
    /// # Errors
    ///
    /// The filesystem's error; the old file is then unchanged.
    pub async fn write(&self, provider: CloudProvider, file: &ManagedKubeconfig) -> OxiResult<()> {
        let text = file.render()?;
        self.fs
            .write_private(&self.path(provider), text.as_bytes())
            .await
    }

    /// Deletes the managed file at `path` (already gone is fine).
    ///
    /// # Errors
    ///
    /// The filesystem's error.
    pub async fn delete(&self, path: &Path) -> OxiResult<()> {
        self.fs.remove(path).await.map(drop)
    }
}

/// The error for a file at a managed path that cloud discovery did not write.
fn not_ours(path: &Path) -> OxiError {
    OxiError::conflict(format!(
        "{} was not written by cloud discovery, so Oxikube will not change it. Rename or remove \
         it to add cloud clusters.",
        path.display()
    ))
}
