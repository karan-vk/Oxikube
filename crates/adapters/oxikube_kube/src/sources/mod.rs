//! Cluster sources: the kubeconfig catalog with hot reload (E03-S02).
//!
//! [`KubeconfigSources`] implements [`ClusterSourcePort`] on top of the tolerant loader
//! ([`crate::kubeconfig`]). It owns the list of sources, the last snapshot of the catalog and
//! the subscribers, and has exactly one code path that reads files: [`reload`](ClusterSourcePort::reload).
//! The file watcher, the safety poll and a manual call all end there.
//!
//! | File | Role |
//! |---|---|
//! | `layout` | expands the configured sources into files; directory filtering |
//! | `snapshot` | the catalog as of one load, and the diff between two |
//! | `watcher` | `notify` on parent directories, debounce, 60 s poll, abort-on-drop |
//! | `pasted` | pasted kubeconfigs, stored in the keychain, never in a file |
//!
//! # Sources
//!
//! In load order (the loader's first-file-wins rule follows it):
//!
//! 1. the files named by `KUBECONFIG`, or the default path (`~/.kube/config`) when that is unset
//!    or empty (kubectl's rule; the caller reads the environment and passes the value in);
//! 2. user-added paths, in the order given. A file is one source; a directory is one source
//!    that expands to the regular files directly inside it (not recursive, sorted by name).
//!    Hidden files and editor leftovers (`.x`, `x~`, `.bak`, `.swp`, `.orig`, `.tmp`, `.lock`)
//!    are skipped; other names are tried and reported as unusable if they are not kubeconfigs;
//! 3. pasted kubeconfigs (the `pasted` module explains where the text lives).
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
//! The pool is not wired here. After each reload, [`KubeconfigSources::kubeconfig`] returns the
//! merged kubeconfig (credentials inside: do not log it) and subscribers receive the
//! [`SourcesChanged`] to invalidate pooled clients for `changed` and `removed` ids.

mod layout;
mod pasted;
mod snapshot;
mod watcher;

#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use kube::config::Kubeconfig;
use oxikube_domain::ids::ContextName;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::secrets::{ExposeSecret, SecretString};
use oxikube_ports::{
    ClusterContext, ClusterSource, ClusterSourcePort, SecretStorePort, SourcesChanged,
};
use parking_lot::{Mutex, RwLock};
use tokio::sync::watch;

use self::layout::Layout;
use self::snapshot::Snapshot;
use crate::kubeconfig::{Diagnostic, Strictness, load_kubeconfig_from_paths_blocking};

pub use self::pasted::PastedDescriptor;

/// Where [`KubeconfigSources`] reads from, and how it watches.
#[derive(Debug, Clone)]
pub struct SourcesConfig {
    /// The value of `KUBECONFIG`, read by the caller (`None` when unset).
    pub kubeconfig_env: Option<OsString>,
    /// `~/.kube/config`, used when `kubeconfig_env` is unset or empty. `None` when the home
    /// directory is unknown.
    pub default_path: Option<PathBuf>,
    /// User-added kubeconfig files and directories, from settings.
    pub extra_paths: Vec<PathBuf>,
    /// Pasted kubeconfigs to restore (the descriptors the caller persisted).
    pub pasted: Vec<PastedDescriptor>,
    /// Start the file watcher and the safety poll. Tests set this to `false`.
    pub watch: bool,
    /// Quiet period after the last file event before reloading.
    pub debounce: Duration,
    /// Interval of the safety poll that catches events the watcher missed.
    pub poll_interval: Duration,
}

impl SourcesConfig {
    /// A configuration with watching on, a 300 ms debounce and a 60 s safety poll.
    pub fn new(kubeconfig_env: Option<OsString>, default_path: Option<PathBuf>) -> Self {
        Self {
            kubeconfig_env,
            default_path,
            extra_paths: Vec::new(),
            pasted: Vec::new(),
            watch: true,
            debounce: Duration::from_millis(300),
            poll_interval: Duration::from_secs(60),
        }
    }
}

/// Whether the file watcher is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchStatus {
    /// `watch` was false: nothing runs besides explicit reloads.
    Disabled,
    /// The watcher is being registered.
    Starting,
    /// Directories are watched and the safety poll runs.
    Active,
    /// The watcher could not start (the reason, without file contents). The safety poll runs,
    /// so changes still appear within one poll interval.
    Failed(String),
}

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
}

impl Inner {
    /// Load every source and build a snapshot. Holds no lock.
    async fn load(&self) -> OxiResult<Snapshot> {
        let config = self.config.clone();
        let pasted = self.pasted.lock().clone();
        let (mut layout, mut loaded) = tokio::task::spawn_blocking(move || {
            let layout = Layout::resolve(&config);
            load_kubeconfig_from_paths_blocking(&layout.files(), Strictness::Tolerant)
                .map(|loaded| (layout, loaded))
        })
        .await
        .map_err(|err| OxiError::internal("kubeconfig source task failed").with_source(err))??;
        pasted::fold_into(&mut loaded, &mut layout, self.secrets.as_ref(), &pasted).await;
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

    pub(crate) fn set_watch_status(&self, status: WatchStatus) {
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

    /// What the last load skipped or shadowed ("file X could not be read"). Empty before the
    /// first load. Updated on every reload, whether or not the catalog changed.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.inner
            .current
            .read()
            .as_ref()
            .map(|s| s.diagnostics.clone())
            .unwrap_or_default()
    }

    /// The merged kubeconfig of the last load, for the client pool. Holds credentials: never
    /// log it. `None` before the first load.
    pub fn kubeconfig(&self) -> Option<Kubeconfig> {
        let current = self.inner.current.read();
        current.as_ref().map(|s| s.loaded.merged.clone())
    }

    /// The kubeconfig's `current-context` as of the last load.
    pub fn current_context(&self) -> Option<ContextName> {
        let current = self.inner.current.read();
        current.as_ref().and_then(|s| s.current_context.clone())
    }

    /// The pasted kubeconfigs, for the caller to persist (they carry no secret).
    pub fn pasted(&self) -> Vec<PastedDescriptor> {
        self.inner.pasted.lock().clone()
    }

    /// Adds a pasted kubeconfig, reloads, and returns its descriptor.
    ///
    /// The text is validated, stored in the keychain through the [`SecretStorePort`] and kept
    /// nowhere else; the descriptor is what to persist. Pasting identical text again replaces
    /// the label. Fails with `Validation` for text that is not a kubeconfig (the message never
    /// quotes it), or with the secret store's error; nothing is written to a file either way.
    pub async fn add_pasted(&self, label: &str, text: SecretString) -> OxiResult<PastedDescriptor> {
        let descriptor = PastedDescriptor::for_text(label, text.expose_secret());
        pasted::parse(text.expose_secret())?;
        self.inner
            .secrets
            .set(&descriptor.secret_key()?, text)
            .await?;
        {
            let mut list = self.inner.pasted.lock();
            match list.iter_mut().find(|d| d.id == descriptor.id) {
                Some(existing) => existing.label = descriptor.label.clone(),
                None => list.push(descriptor.clone()),
            }
        }
        self.inner.reload().await?;
        Ok(descriptor)
    }

    /// Removes a pasted kubeconfig from the keychain and the catalog and reloads. Returns
    /// whether `id` was known.
    pub async fn remove_pasted(&self, id: &str) -> OxiResult<bool> {
        let Some(descriptor) = self
            .inner
            .pasted
            .lock()
            .iter()
            .find(|d| d.id == id)
            .cloned()
        else {
            return Ok(false);
        };
        // Delete the secret first: if the keychain refuses, the entry stays listed and retryable.
        self.inner.secrets.delete(&descriptor.secret_key()?).await?;
        self.inner.pasted.lock().retain(|d| d.id != id);
        self.inner.reload().await?;
        Ok(true)
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
