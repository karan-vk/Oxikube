//! The adapter's configuration and watcher status.

use std::path::PathBuf;
use std::time::Duration;

use super::PastedDescriptor;
use crate::kubeconfig::Env;

/// Where [`KubeconfigSources`](super::KubeconfigSources) reads from, and how it watches.
#[derive(Debug, Clone)]
pub struct SourcesConfig {
    /// `KUBECONFIG`, the home directory and the in-cluster inputs, read by the caller
    /// ([`Env::from_process`] in the app, off the UI thread: it stats files). Picks the kubectl tier ([`select_sources`]) and
    /// decides the in-cluster fallback ([`apply_in_cluster_fallback`]).
    ///
    /// [`select_sources`]: crate::kubeconfig::select_sources
    /// [`apply_in_cluster_fallback`]: crate::kubeconfig::apply_in_cluster_fallback
    pub env: Env,
    /// User-added kubeconfig files and directories, from settings. Additive: loaded after the
    /// `KUBECONFIG` or default-path files, not instead of them.
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
    pub fn new(env: Env) -> Self {
        Self {
            env,
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
