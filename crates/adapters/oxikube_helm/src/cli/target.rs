//! [`HelmTarget`] and [`TargetResolver`]: which kubeconfig context a cluster's `helm` commands
//! talk to.

use std::path::PathBuf;

use oxikube_domain::ids::{ClusterId, ContextName};

/// Which cluster `helm` talks to: a context name and, when known, the kubeconfig file that
/// defines it. Names and paths only, never credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelmTarget {
    /// `--kube-context`.
    pub context: ContextName,
    /// `--kubeconfig`, when the file that defines the context is known; otherwise `helm`
    /// reads its default kubeconfig (`KUBECONFIG` or `~/.kube/config`).
    pub kubeconfig: Option<PathBuf>,
}

impl HelmTarget {
    /// A target for `context` in `kubeconfig`.
    pub fn new(context: ContextName, kubeconfig: Option<PathBuf>) -> Self {
        Self {
            context,
            kubeconfig,
        }
    }

    /// The global flags that select this target.
    pub(crate) fn args(&self) -> Vec<String> {
        let mut args = vec![format!("--kube-context={}", self.context)];
        if let Some(path) = &self.kubeconfig {
            args.push(format!("--kubeconfig={}", path.display()));
        }
        args
    }
}

/// Maps a session's [`ClusterId`] to its [`HelmTarget`]. The binary wires one that reads the
/// kubeconfig catalog (the same origin lookup `kubectl describe` uses).
///
/// Implemented for any `Fn(&ClusterId) -> Option<HelmTarget>`.
pub trait TargetResolver: Send + Sync {
    /// The target of `cluster`, `None` when the cluster is unknown.
    fn target(&self, cluster: &ClusterId) -> Option<HelmTarget>;
}

impl<F> TargetResolver for F
where
    F: Fn(&ClusterId) -> Option<HelmTarget> + Send + Sync,
{
    fn target(&self, cluster: &ClusterId) -> Option<HelmTarget> {
        self(cluster)
    }
}
