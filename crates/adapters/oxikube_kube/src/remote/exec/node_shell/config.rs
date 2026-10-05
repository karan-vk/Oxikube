//! Settings of a node shell.

use std::time::Duration;

/// Default image for the shell pod: small, and its `nsenter` (busybox) enters the node's
/// namespaces. Configurable because clusters often cannot reach Docker Hub (the setting is
/// `node shell image` in E06-S08).
pub const DEFAULT_IMAGE: &str = "busybox:1.37";

/// Default namespace for the shell pod: it exists on every cluster and is exempt from the
/// pod security admission that blocks privileged pods in ordinary namespaces.
pub const DEFAULT_NAMESPACE: &str = "kube-system";

/// How the node shell pod is built and how long it may take to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeShellConfig {
    /// Namespace of the shell pod.
    pub namespace: String,
    /// Image of the shell pod; it needs `nsenter` and `sleep`.
    pub image: String,
    /// Name of an image pull secret in `namespace`, for a private registry.
    pub image_pull_secret: Option<String>,
    /// The command run inside the node's namespaces. Empty runs the node's `bash` if it has
    /// one, else its `sh`, as a login shell.
    pub shell: Vec<String>,
    /// The longest the pod lives, enforced by the cluster (`activeDeadlineSeconds`): a safety
    /// net for when neither the cleanup nor the leftover sweep got to delete it.
    pub max_lifetime: Duration,
    /// How long to wait for the pod to run (scheduling is skipped, the image pull is not).
    pub start_timeout: Duration,
}

impl Default for NodeShellConfig {
    fn default() -> Self {
        Self {
            namespace: DEFAULT_NAMESPACE.into(),
            image: DEFAULT_IMAGE.into(),
            image_pull_secret: None,
            shell: Vec::new(),
            max_lifetime: Duration::from_secs(8 * 60 * 60),
            start_timeout: Duration::from_secs(120),
        }
    }
}
