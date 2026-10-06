//! The kubeconfig a cluster terminal's shell reads: a private, minimal file holding exactly the
//! selected context, written to a per-process runtime directory and deleted with the terminal.
//!
//! The shell gets `KUBECONFIG` pointing at it, so `kubectl`, `helm` and `argocd` talk to the
//! cluster the tab shows without `--context`, and the user's own kubeconfig files (which may
//! hold every other cluster's credentials) are not handed to the shell wholesale. The file can
//! carry inline credentials the source file already had; it is `0600` inside a `0700`
//! directory, never logged, and removed when the terminal ends (or, after a crash, by the next
//! start's sweep).

mod merge;
mod runtime_dir;

use std::path::{Path, PathBuf};

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ContextName;
use oxikube_ports::cluster_source::{ClusterSource, SourceKind};

pub use merge::merged_kubeconfig;
pub use runtime_dir::{TempKubeconfig, cleanup_runtime_dir};

/// The cluster a terminal is opened for: what the shell's environment is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterEnv {
    /// The kubeconfig context (`KUBE_CONTEXT`, and the merged file's `current-context`).
    pub context: ContextName,
    /// The namespace the terminal starts in (`OXIKUBE_NAMESPACE`, and the merged context's
    /// namespace). `None` uses the context's own namespace, then `default`.
    pub namespace: Option<String>,
    /// The kubeconfig files to read the context from, in load order: the first definition of a
    /// name wins, as in kubectl. See [`files_of_sources`].
    pub kubeconfig_files: Vec<PathBuf>,
}

impl ClusterEnv {
    /// An environment for `context` read from `kubeconfig_files`.
    pub fn new(context: ContextName, kubeconfig_files: Vec<PathBuf>) -> Self {
        Self {
            context,
            namespace: None,
            kubeconfig_files,
        }
    }

    /// Starts the terminal in `namespace`.
    #[must_use]
    pub fn in_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    /// Writes the merged kubeconfig for this cluster to the runtime directory and returns it
    /// with the variables to set in the shell.
    ///
    /// Blocking (reads the kubeconfig files, writes one file): call it off the UI thread.
    ///
    /// # Errors
    ///
    /// `NotFound` when no file defines the context, `Validation` when a file defining it is not a
    /// kubeconfig, `Internal` for I/O failures. Messages never quote file content.
    pub fn prepare(&self) -> OxiResult<PreparedEnv> {
        let merged = merged_kubeconfig(self)?;
        let file = TempKubeconfig::write(merged.text.as_bytes())?;
        let vars = vec![
            ("KUBECONFIG", file.path().to_string_lossy().into_owned()),
            ("KUBE_CONTEXT", self.context.as_str().to_owned()),
            ("OXIKUBE_NAMESPACE", merged.namespace),
        ];
        Ok(PreparedEnv { vars, file })
    }
}

/// A prepared cluster environment: the variables for the shell and the temp file they point at
/// (deleted when this is dropped).
#[derive(Debug)]
pub struct PreparedEnv {
    /// The variables to set. The values are paths and names, never credentials.
    pub vars: Vec<(&'static str, String)>,
    /// The merged kubeconfig; keep it alive as long as the shell runs.
    pub file: TempKubeconfig,
}

/// The kubeconfig files behind catalog sources, in source order: a file source is itself, a
/// directory source is the regular files directly inside it (sorted by name). Sources with no
/// path (the in-cluster account) contribute nothing.
pub fn files_of_sources(sources: &[ClusterSource]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for source in sources {
        let Some(path) = source.path.as_deref() else {
            continue;
        };
        match source.kind {
            SourceKind::KubeconfigFile | SourceKind::Environment => files.push(path.to_owned()),
            SourceKind::KubeconfigDir => files.extend(files_in(path)),
            SourceKind::InCluster | SourceKind::Cloud => {}
        }
    }
    files
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests;
