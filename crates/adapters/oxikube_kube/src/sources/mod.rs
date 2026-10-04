//! Cluster sources: the kubeconfig catalog with hot reload (E03-S02).
//!
//! [`KubeconfigSources`] implements [`ClusterSourcePort`] on top of the tolerant loader
//! ([`crate::kubeconfig`]). It owns the list of sources, the last snapshot of the catalog and
//! the subscribers, and has exactly one code path that reads files: [`reload`](ClusterSourcePort::reload).
//! The file watcher, the safety poll and a manual call all end there.
//!
//! | File | Role |
//! |---|---|
//! | `config` | [`SourcesConfig`] and [`WatchStatus`] |
//! | `layout` | expands the configured sources into files; directory filtering |
//! | `snapshot` | the catalog as of one load, and the diff between two |
//! | `watcher` | `notify` on parent directories, debounce, 60 s poll, abort-on-drop |
//! | `pasted` | pasted kubeconfigs, stored in the keychain, never in a file |
//!
//! # Sources
//!
//! In load order (the loader's first-file-wins rule follows it):
//!
//! 1. the kubectl tier picked by [`select_sources`](crate::kubeconfig::select_sources) from
//!    [`SourcesConfig::env`]: the files named by `KUBECONFIG` when it is set and non-empty (even
//!    when it splits to no path, as with `KUBECONFIG=":"`, in which case nothing is read here),
//!    otherwise the default path (`<home>/.kube/config`);
//! 2. user-added paths, in the order given. These are additive (loaded after tier 1), unlike
//!    the loader's exclusive "explicit" tier. A file is one source; a directory is one source
//!    that expands to the regular files directly inside it (not recursive, sorted by name).
//!    Hidden files and editor leftovers (`.x`, `x~`, `.bak`, `.swp`, `.orig`, `.tmp`, `.lock`)
//!    are skipped; other names are tried and reported as unusable if they are not kubeconfigs;
//! 3. pasted kubeconfigs (the `pasted` module explains where the text lives);
//! 4. the pod's service account, as a [`SourceKind::InCluster`](oxikube_ports::SourceKind)
//!    source holding the synthetic `in-cluster` context, only when 1-3 gave no context and no
//!    file was broken ([`apply_in_cluster_fallback`]).
//!
//! The source list is fixed at construction apart from pasted kubeconfigs. Changing the
//! user-added paths at runtime means building a new adapter (the settings UI is E06).
//!
//! # Change detection
//!
//! A reload builds a snapshot (`snapshot` module) and compares it with the last one. Entries are
//! compared by a SHA-256 of the parsed context, cluster and user, so rewriting a file with
//! identical content, or touching it, emits nothing. Reported as `changed`: a new server,
//! namespace, source, user or credentials, and the two contexts involved when the kubeconfig's
//! `current-context` changes (the port's `ClusterContext` has no field for it; read
//! [`KubeconfigSources::current_context`]).
//!
//! # Threading
//!
//! Nothing here runs on the caller's thread beyond channel sends: file work is on tokio's
//! blocking pool, the watcher's own thread only sends `()` into a channel, and reloads are
//! serialised so subscribers see diffs in order. Construct inside a tokio runtime when
//! [`SourcesConfig::watch`] is true; with `watch: false` no thread or task is started and tests
//! drive everything with `reload()`.
//!
//! # For the client pool (E03-S03)
//!
//! The pool is not owned here; the wiring drives it. After each [`SourcesChanged`] (or any
//! `reload()`), pass [`KubeconfigSources::loaded`] to
//! [`ClientPool::replace_loaded`](crate::ClientPool::replace_loaded). The pool compares each
//! pooled context's connection definition with the new kubeconfig and drops only the clients
//! of contexts that changed or vanished; the diff itself is for the UI.
//!
//! ```ignore
//! let mut changes = sources.subscribe();
//! while changes.next().await.is_some() {
//!     if let Some(loaded) = sources.loaded() {
//!         pool.replace_loaded(&loaded);
//!     }
//! }
//! ```

mod config;
mod layout;
mod pasted;
mod snapshot;
mod watcher;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use oxikube_domain::ids::ContextName;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::secrets::SecretString;
use oxikube_ports::{
    ClusterContext, ClusterSource, ClusterSourcePort, SecretStorePort, SourcesChanged,
};
use parking_lot::{Mutex, RwLock};
use tokio::sync::watch;

use self::layout::Layout;
use self::snapshot::Snapshot;
use crate::kubeconfig::{Diagnostic, KubeconfigMerge, LoadedKubeconfig, apply_in_cluster_fallback};

pub use self::config::{SourcesConfig, WatchStatus};
pub use self::pasted::PastedDescriptor;

/// Shared state; the watch task holds a [`std::sync::Weak`] to it.
pub(crate) struct Inner {
    config: SourcesConfig,
    secrets: Arc<dyn SecretStorePort>,
    pasted: Mutex<Vec<PastedDescriptor>>,
    /// Serialises loads so diffs are computed and emitted in order.
    reload_lock: tokio::sync::Mutex<()>,
    current: RwLock<Option<Arc<Snapshot>>>,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<SourcesChanged>>>,
    watch_status: watch::Sender<WatchStatus>,
    /// Directories the watcher could not register (the poll covers them).
    unwatched: Mutex<Vec<PathBuf>>,
    /// Pasted kubeconfig text by descriptor id, read from the keychain once. Reloads are
    /// frequent and a keychain read can be slow or prompt; the text is already in memory
    /// whenever it is parsed, and `SecretString` wipes it on drop.
    pasted_text: Mutex<HashMap<String, SecretString>>,
}

impl Inner {
    /// Load every source and build a snapshot. Holds no lock.
    ///
    /// Files first (on the blocking pool), then pasted kubeconfigs, then the in-cluster
    /// fallback, which only applies when neither gave a context and nothing was broken.
    async fn load(&self) -> OxiResult<Snapshot> {
        let config = self.config.clone();
        let pasted = self.pasted.lock().clone();
        let (mut layout, mut merge) = tokio::task::spawn_blocking(move || {
            let layout = Layout::resolve(&config);
            let mut merge = KubeconfigMerge::new();
            for file in layout.files() {
                merge.add_file(&file);
            }
            (layout, merge)
        })
        .await
        .map_err(|err| OxiError::internal("kubeconfig source task failed").with_source(err))?;
        pasted::fold_into(
            &mut merge,
            &mut layout,
            self.secrets.as_ref(),
            &pasted,
            &self.pasted_text,
        )
        .await;
        let mut loaded = merge.finish();
        apply_in_cluster_fallback(&mut loaded, &self.config.env)?;
        if loaded.sources.iter().any(|s| s.is_in_cluster()) {
            layout.push_in_cluster();
        }
        Ok(Snapshot::build(loaded, layout))
    }

    /// The single code path that re-reads sources: load, diff against the last snapshot,
    /// commit, and notify subscribers when the diff is not empty.
    pub(crate) async fn reload(&self) -> OxiResult<SourcesChanged> {
        let _serial = self.reload_lock.lock().await;
        let snapshot = Arc::new(self.load().await?);
        let previous = self.current.write().replace(snapshot.clone());
        let diff = snapshot.diff_from(previous.as_deref());
        if !diff.is_empty() {
            self.subscribers
                .lock()
                .retain(|tx| tx.unbounded_send(diff.clone()).is_ok());
        }
        Ok(diff)
    }

    /// The last snapshot, loading it (without notifying anyone) if there is none yet.
    async fn snapshot(&self) -> OxiResult<Arc<Snapshot>> {
        if let Some(snapshot) = self.current.read().clone() {
            return Ok(snapshot);
        }
        let _serial = self.reload_lock.lock().await;
        if let Some(snapshot) = self.current.read().clone() {
            return Ok(snapshot);
        }
        let snapshot = Arc::new(self.load().await?);
        *self.current.write() = Some(snapshot.clone());
        Ok(snapshot)
    }

    pub(crate) fn set_watch_status(&self, status: WatchStatus, unwatched: Vec<PathBuf>) {
        *self.unwatched.lock() = unwatched;
        self.watch_status.send_replace(status);
    }
}

/// The kubeconfig-backed [`ClusterSourcePort`].
///
/// Dropping it stops the watcher. Cheap to share behind an `Arc`.
pub struct KubeconfigSources {
    inner: Arc<Inner>,
    _watch: Option<watcher::WatchGuard>,
}

impl fmt::Debug for KubeconfigSources {
    /// Counts only: the snapshot holds credentials.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KubeconfigSources")
            .field("pasted", &self.inner.pasted.lock().len())
            .field("watch", &*self.inner.watch_status.borrow())
            .finish_non_exhaustive()
    }
}

impl KubeconfigSources {
    /// Build the adapter. Nothing is read until the first port call or [`reload`](ClusterSourcePort::reload).
    ///
    /// `secrets` stores pasted kubeconfig text. With `config.watch` the watcher starts, which
    /// needs a tokio runtime (an error otherwise).
    pub fn new(config: SourcesConfig, secrets: Arc<dyn SecretStorePort>) -> OxiResult<Self> {
        let initial = if config.watch {
            WatchStatus::Starting
        } else {
            WatchStatus::Disabled
        };
        let inner = Arc::new(Inner {
            pasted: Mutex::new(config.pasted.clone()),
            config,
            secrets,
            reload_lock: tokio::sync::Mutex::new(()),
            current: RwLock::new(None),
            subscribers: Mutex::new(Vec::new()),
            watch_status: watch::channel(initial).0,
            unwatched: Mutex::new(Vec::new()),
            pasted_text: Mutex::new(HashMap::new()),
        });
        let guard = if inner.config.watch {
            Some(watcher::spawn(Arc::downgrade(&inner), &inner.config)?)
        } else {
            None
        };
        Ok(Self {
            inner,
            _watch: guard,
        })
    }

    /// Whether the watcher is running (see [`wait_for_watcher`](Self::wait_for_watcher)).
    pub fn watch_status(&self) -> WatchStatus {
        self.inner.watch_status.borrow().clone()
    }

    /// Resolves once the watcher has registered its directories or failed to, so a caller that
    /// is about to change a file knows the change will be seen.
    pub async fn wait_for_watcher(&self) -> WatchStatus {
        let mut status = self.inner.watch_status.subscribe();
        let settled = status
            .wait_for(|s| *s != WatchStatus::Starting)
            .await
            .map(|s| s.clone());
        settled.unwrap_or_else(|_| status.borrow().clone())
    }

    /// What the last load skipped or shadowed ("file X could not be read"), plus directories
    /// the watcher could not register. Empty before the first load. Updated on every reload,
    /// whether or not the catalog changed.
    ///
    /// Not part of [`ClusterSourcePort`]: callers holding the port as a trait object cannot
    /// reach it until the port grows a diagnostics method.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut found: Vec<Diagnostic> = self
            .inner
            .current
            .read()
            .iter()
            .flat_map(|s| s.diagnostics.iter().cloned())
            .collect();
        found.extend(
            self.inner
                .unwatched
                .lock()
                .iter()
                .map(|path| Diagnostic::Unreadable {
                    path: path.clone(),
                    reason: "could not be watched for changes; checked every poll interval".into(),
                }),
        );
        found
    }

    /// The loader result of the last load, for the client pool (`None` before the first load).
    ///
    /// Hand it to [`ClientPool::replace_loaded`](crate::ClientPool::replace_loaded) after each
    /// [`SourcesChanged`]: the pool compares connection definitions and drops only the clients
    /// of contexts that changed or vanished, and it learns which context is the synthetic
    /// in-cluster one. The merged config holds credentials: never log it.
    pub fn loaded(&self) -> Option<Arc<LoadedKubeconfig>> {
        let current = self.inner.current.read();
        current.as_ref().map(|s| s.loaded.clone())
    }

    /// The kubeconfig's `current-context` as of the last load.
    pub fn current_context(&self) -> Option<ContextName> {
        let current = self.inner.current.read();
        current.as_ref().and_then(|s| s.current_context.clone())
    }
}

#[async_trait]
impl ClusterSourcePort for KubeconfigSources {
    async fn sources(&self) -> OxiResult<Vec<ClusterSource>> {
        Ok(self.inner.snapshot().await?.sources.clone())
    }

    async fn contexts(&self) -> OxiResult<Vec<ClusterContext>> {
        Ok(self.inner.snapshot().await?.contexts())
    }

    fn subscribe(&self) -> BoxStream<'static, SourcesChanged> {
        let (tx, rx) = mpsc::unbounded();
        self.inner.subscribers.lock().push(tx);
        rx.boxed()
    }

    async fn reload(&self) -> OxiResult<SourcesChanged> {
        self.inner.reload().await
    }
}
