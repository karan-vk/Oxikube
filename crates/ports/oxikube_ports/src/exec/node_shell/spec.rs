//! [`NodeShellSpec`]: what a node shell's helper pod is made from.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::cluster_prefs::ClusterPrefs;

/// Default image of the helper pod: small, and its `nsenter` (busybox) enters the node's
/// namespaces. Configurable (`node_shell_image`) because clusters often cannot reach Docker Hub.
pub const DEFAULT_NODE_SHELL_IMAGE: &str = "busybox:1.37";

/// Default namespace of the helper pod: it exists on every cluster and is exempt from the pod
/// security admission that blocks privileged pods in ordinary namespaces. Configurable
/// (`node_shell.namespace`).
pub const DEFAULT_NODE_SHELL_NAMESPACE: &str = "kube-system";

/// The `nsenter` options that enter every namespace of the node's init process: mount, UTS, IPC,
/// network and PID. Configurable (`node_shell.nsenter_args`).
pub const DEFAULT_NSENTER_ARGS: [&str; 7] = ["-t", "1", "-m", "-u", "-i", "-n", "-p"];

/// The longest the helper pod lives unless the settings say otherwise: enforced by the cluster
/// (`activeDeadlineSeconds`), the safety net behind the cleanup and the leftover sweep.
const DEFAULT_MAX_LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);

/// How long to wait for the helper pod to run (the image pull is included).
const DEFAULT_START_TIMEOUT: Duration = Duration::from_secs(120);

/// One toleration of the helper pod (a Kubernetes `Toleration`). Every field is optional like
/// the original: `{ "operator": "Exists" }` tolerates every taint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeShellToleration {
    /// The taint key; `None` with operator `Exists` matches every key.
    pub key: Option<String>,
    /// `Exists` or `Equal` (the server's default).
    pub operator: Option<String>,
    /// The taint value, for `Equal`.
    pub value: Option<String>,
    /// `NoSchedule`, `PreferNoSchedule` or `NoExecute`; `None` matches every effect.
    pub effect: Option<String>,
    /// For `NoExecute`: how long the pod stays after the taint appears.
    pub toleration_seconds: Option<i64>,
}

impl NodeShellToleration {
    /// The toleration that matches every taint, so the pod runs on a tainted or cordoned node.
    pub fn everything() -> Self {
        Self {
            operator: Some("Exists".to_owned()),
            ..Self::default()
        }
    }
}

/// A shell on a node through a privileged helper pod
/// ([`ExecPort::node_shell`](crate::ExecPort::node_shell)).
///
/// [`NodeShellSpec::new`] carries every default; [`NodeShellSpec::for_node`] fills the template
/// from a cluster's settings. The pod it renders to is [`node_shell_manifest`](super::node_shell_manifest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeShellSpec {
    /// The node's name.
    pub node: String,
    /// Image of the helper pod; needs `nsenter` (and `sleep`).
    pub image: String,
    /// Namespace of the helper pod.
    pub namespace: String,
    /// Name of an image pull secret in `namespace`, for a private registry.
    pub image_pull_secret: Option<String>,
    /// `Always`, `IfNotPresent` or `Never`; `None` leaves the cluster's default.
    pub image_pull_policy: Option<String>,
    /// The `nsenter` options before the `--`; [`DEFAULT_NSENTER_ARGS`] by default (and when
    /// emptied).
    pub nsenter_args: Vec<String>,
    /// The command run inside the node's namespaces; empty runs the node's `bash` if it has one,
    /// else its `sh`, as a login shell.
    pub shell: Vec<String>,
    /// What the pod tolerates. The pod is pinned to its node by name, so the scheduler is not
    /// involved, but a `NoExecute` taint would still evict it.
    pub tolerations: Vec<NodeShellToleration>,
    /// Labels added to the pod. They cannot replace Oxikube's own (`app.kubernetes.io/managed-by`
    /// and the node-shell marker the leftover sweep selects on).
    pub labels: BTreeMap<String, String>,
    /// The longest the pod lives, enforced by the cluster: a safety net for when neither the
    /// cleanup nor the leftover sweep got to delete it.
    pub max_lifetime: Duration,
    /// How long to wait for the pod to run.
    pub start_timeout: Duration,
}

impl NodeShellSpec {
    /// A node shell on `node` with every default.
    pub fn new(node: impl Into<String>) -> Self {
        Self {
            node: node.into(),
            image: DEFAULT_NODE_SHELL_IMAGE.to_owned(),
            namespace: DEFAULT_NODE_SHELL_NAMESPACE.to_owned(),
            image_pull_secret: None,
            image_pull_policy: None,
            nsenter_args: DEFAULT_NSENTER_ARGS.map(String::from).into(),
            shell: Vec::new(),
            tolerations: vec![NodeShellToleration::everything()],
            labels: BTreeMap::new(),
            max_lifetime: DEFAULT_MAX_LIFETIME,
            start_timeout: DEFAULT_START_TIMEOUT,
        }
    }

    /// The template of `prefs` (a cluster's resolved settings) for a shell on `node`: the
    /// settings that are set over [`NodeShellSpec::new`]'s defaults.
    pub fn for_node(node: impl Into<String>, prefs: &ClusterPrefs) -> Self {
        let mut spec = Self::new(node);
        let node_shell = &prefs.node_shell;
        if let Some(image) = &prefs.node_shell_image {
            spec.image.clone_from(image);
        }
        if let Some(namespace) = &node_shell.namespace {
            spec.namespace.clone_from(namespace);
        }
        spec.image_pull_secret
            .clone_from(&prefs.node_shell_pull_secret);
        spec.image_pull_policy
            .clone_from(&node_shell.image_pull_policy);
        if !node_shell.nsenter_args.is_empty() {
            spec.nsenter_args.clone_from(&node_shell.nsenter_args);
        }
        spec.shell.clone_from(&node_shell.command);
        if let Some(tolerations) = &node_shell.tolerations {
            spec.tolerations.clone_from(tolerations);
        }
        spec.labels.clone_from(&node_shell.labels);
        if let Some(seconds) = node_shell.max_lifetime_seconds {
            spec.max_lifetime = Duration::from_secs(seconds);
        }
        spec
    }
}
