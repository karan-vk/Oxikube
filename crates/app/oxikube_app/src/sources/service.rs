//! [`KubeconfigSourcesService`]: add, paste, remove and reload kubeconfig sources.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::lock::Mutex;
use oxikube_domain::command::{KubeconfigSourceRef, NewKubeconfigSource};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{ClusterSourcePort, FsPort, SourcesChanged, UserSource, UserSourceKind};

use super::list::SourceListStore;
use super::name;
use super::row::{self, SourceRow};

/// What a change to the list did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceChange {
    /// The entry that was added or removed.
    pub source: UserSource,
    /// The entry was already in the list (adding) so nothing changed.
    pub unchanged: bool,
    /// A pasted kubeconfig was written to this path.
    pub created_file: Option<PathBuf>,
    /// Oxikube's own copy of a kubeconfig was deleted from this path.
    pub deleted_file: Option<PathBuf>,
    /// How many contexts a pasted text defines (validation result); zero otherwise.
    pub contexts_in_paste: usize,
}

impl SourceChange {
    fn new(source: UserSource) -> Self {
        Self {
            source,
            unchanged: false,
            created_file: None,
            deleted_file: None,
            contexts_in_paste: 0,
        }
    }
}

/// Manages the user's kubeconfig sources. Cheap to clone; clones share one lock, so two
/// changes to the list never overwrite each other.
///
/// See the [module docs](super) for the rules.
#[derive(Clone)]
pub struct KubeconfigSourcesService {
    source: Arc<dyn ClusterSourcePort>,
    fs: Arc<dyn FsPort>,
    list: Arc<dyn SourceListStore>,
    /// `<config dir>/kubeconfigs`: where pasted kubeconfigs are stored.
    dir: PathBuf,
    /// Held across a read-modify-write of the list.
    write: Arc<Mutex<()>>,
}

impl std::fmt::Debug for KubeconfigSourcesService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubeconfigSourcesService")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl KubeconfigSourcesService {
    /// A service over the cluster source, the filesystem and the stored list. Pasted
    /// kubeconfigs are stored in `kubeconfigs_dir` (`<config dir>/kubeconfigs`).
    pub fn new(
        source: Arc<dyn ClusterSourcePort>,
        fs: Arc<dyn FsPort>,
        list: Arc<dyn SourceListStore>,
        kubeconfigs_dir: PathBuf,
    ) -> Self {
        Self {
            source,
            fs,
            list,
            dir: kubeconfigs_dir,
            write: Arc::default(),
        }
    }

    /// The directory pasted kubeconfigs are stored in.
    pub fn kubeconfigs_dir(&self) -> &Path {
        &self.dir
    }

    /// Whether removing `source` deletes a file: true for a kubeconfig stored in
    /// [`kubeconfigs_dir`](Self::kubeconfigs_dir), false for a file or folder the user keeps
    /// elsewhere. The confirmation text depends on it. Lexical, no file access.
    pub fn deletes_file_on_remove(&self, source: &UserSource) -> bool {
        source.kind == UserSourceKind::File
            && source
                .path
                .as_deref()
                .is_some_and(|path| name::is_stored_in(&self.dir, path))
    }

    /// The list with how each source was read. Local: kubeconfig files and nothing else.
    ///
    /// # Errors
    ///
    /// The stored list's or the cluster source's error.
    pub async fn rows(&self) -> OxiResult<Vec<SourceRow>> {
        let (list, statuses) = futures::join!(self.list.load(), self.source.source_statuses());
        let list = list?;
        let statuses = statuses?;
        Ok(list
            .iter()
            .map(|source| row::build(source, self.deletes_file_on_remove(source), &statuses))
            .collect())
    }

    /// Tells the cluster source about the stored list and reads every source. Run it when the
    /// list changed outside this service (the user edited `settings.json`) and once at start.
    ///
    /// # Errors
    ///
    /// The stored list's or the cluster source's error.
    pub async fn apply_stored(&self) -> OxiResult<SourcesChanged> {
        let list = self.list.load().await?;
        self.source.set_user_sources(&list).await
    }

    /// The catalog's changes from now on (the cluster source's diffs, unchanged). A new read
    /// of the sources that changes no context sends nothing, so refresh [`rows`](Self::rows)
    /// after your own commands too.
    pub fn changes(&self) -> futures::stream::BoxStream<'static, SourcesChanged> {
        self.source.subscribe()
    }

    /// Reads every source again now.
    ///
    /// # Errors
    ///
    /// The cluster source's error. A broken kubeconfig file is not an error: it shows in the
    /// [`rows`](Self::rows).
    pub async fn reload(&self) -> OxiResult<SourcesChanged> {
        self.source.reload().await
    }

    /// Adds a source: a file or folder (the entry only), the default entry, or a pasted
    /// kubeconfig (validated, stored as `<kubeconfigs dir>/<name>.yaml` owner-only, then
    /// listed as a file).
    ///
    /// A file or folder that is missing or not a kubeconfig is still added: its row shows the
    /// problem and the other sources keep loading. Pasted text is checked first and nothing is
    /// written when it is not a kubeconfig.
    ///
    /// # Errors
    ///
    /// `Validation` for an empty or relative path, a bad name, or text that is not a
    /// kubeconfig; `Conflict` when a stored kubeconfig of that name exists; the filesystem's,
    /// the list's or the cluster source's error. A failed step leaves no half-added source: a
    /// file written for a paste is deleted again when the list cannot be saved.
    pub async fn add(&self, new: &NewKubeconfigSource) -> OxiResult<SourceChange> {
        let _write = self.write.lock().await;
        let mut list = self.list.load().await?;
        let (entry, created, contexts) = match new {
            NewKubeconfigSource::Default => (UserSource::default_source(), None, 0),
            NewKubeconfigSource::File { path } => (UserSource::file(checked_path(path)?), None, 0),
            NewKubeconfigSource::Dir { path } => (UserSource::dir(checked_path(path)?), None, 0),
            NewKubeconfigSource::Pasted { name, text } => {
                let file_name = name::pasted_file_name(name)?;
                let contexts = self.source.validate_kubeconfig(text.expose()).await?;
                let path = self.dir.join(&file_name);
                self.ensure_free(&file_name).await?;
                self.fs
                    .write_private(&path, text.expose().as_bytes())
                    .await?;
                (UserSource::file(path.clone()), Some(path), contexts)
            }
        };
        let mut change = SourceChange::new(entry.clone());
        change.created_file = created.clone();
        change.contexts_in_paste = contexts;
        if list.contains(&entry) {
            change.unchanged = true;
        } else {
            list.push(entry);
            if let Err(error) = self.list.save(&list).await {
                if let Some(path) = &created {
                    self.discard(path).await;
                }
                return Err(error);
            }
        }
        self.source.set_user_sources(&list).await?;
        Ok(change)
    }

    /// Removes a source from the list. A kubeconfig stored in the kubeconfigs directory (a
    /// pasted one) is deleted with its entry; a file or folder the user keeps elsewhere is
    /// only taken off the list, never deleted.
    ///
    /// # Errors
    ///
    /// `NotFound` when the list has no such entry; the filesystem's error when the stored file
    /// cannot be deleted (the entry stays, so the removal can be retried); the list's or the
    /// cluster source's error.
    pub async fn remove(&self, which: &KubeconfigSourceRef) -> OxiResult<SourceChange> {
        let _write = self.write.lock().await;
        let mut list = self.list.load().await?;
        let target = match which {
            KubeconfigSourceRef::Default => UserSource::default_source(),
            KubeconfigSourceRef::File { path } => UserSource::file(checked_path(path)?),
            KubeconfigSourceRef::Dir { path } => UserSource::dir(checked_path(path)?),
        };
        let Some(position) = list.iter().position(|s| *s == target) else {
            return Err(OxiError::not_found(
                "That kubeconfig source is not in the list.",
            ));
        };
        let mut change = SourceChange::new(target.clone());
        if self.deletes_file_on_remove(&target)
            && let Some(path) = target.path.as_deref()
        {
            // First, so that a file that cannot be deleted keeps its entry and the user can
            // try again; a file that is already gone is fine.
            self.fs.remove(path).await?;
            change.deleted_file = Some(path.to_owned());
        }
        list.remove(position);
        self.list.save(&list).await?;
        self.source.set_user_sources(&list).await?;
        Ok(change)
    }

    /// `Conflict` when a stored kubeconfig with the same name (any letter case) exists.
    async fn ensure_free(&self, file_name: &str) -> OxiResult<()> {
        let existing = match self.fs.list(&self.dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let taken = existing.iter().any(|entry| {
            entry
                .path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(file_name))
        });
        if taken {
            return Err(OxiError::conflict(format!(
                "A kubeconfig named {} already exists. Pick another name, or remove it first.",
                file_name.trim_end_matches(".yaml")
            )));
        }
        Ok(())
    }

    /// Deletes a file written by a change that then failed. Best effort: the failure that got
    /// us here is the one worth reporting.
    async fn discard(&self, path: &Path) {
        if let Err(error) = self.fs.remove(path).await {
            tracing::warn!(%error, path = %path.display(), "could not delete a kubeconfig that was not added");
        }
    }
}

/// A source path from a command: not empty and absolute. A relative path would mean something
/// different depending on where Oxikube was started.
fn checked_path(path: &str) -> OxiResult<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        return Err(OxiError::validation("Choose a file or folder."));
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(OxiError::validation(
            "Use the full path of the file or folder (it starts with / or a drive letter).",
        ));
    }
    Ok(path)
}
