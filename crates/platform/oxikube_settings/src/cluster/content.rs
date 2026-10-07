//! What one settings layer may say about a cluster: the file shape of the per-cluster keys.
//!
//! These are root-level keys of `settings.json`, so the same field set is valid at the top of
//! the file (a default for every cluster) and inside `clusters.<id>` (an override for one).
//! Layers merge field by field: `default.json`, then the user's top-level value, then the
//! cluster's own.

use oxikube_domain::ClusterColour;
use oxikube_ports::ExecInteractivity;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::fields::{HttpUrl, SecretName, colour_schema, exec_interactivity_schema};
use super::node_shell::NodeShellContent;
use super::watch_budget::WatchBudgetContent;

/// A manual Prometheus location for a cluster (`prometheus` key). Keeps no secret: the bearer
/// token lives in the OS keychain and `auth_secret` only names its entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PrometheusContent {
    /// `auto` (probe the cluster) or a provider id such as `operator`, `lens`, `helm`,
    /// `victoria_metrics`, `mimir`. Unset means auto-detect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// `namespace/service:port`, reached through the API server's service proxy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// A direct `http(s)` URL, used instead of `path`. No credentials, query or fragment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<HttpUrl>,
    /// Name of the keychain entry (namespace `prometheus`) that holds the bearer token. Never
    /// put the token itself in `settings.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_secret: Option<SecretName>,
}

/// What one settings layer says about a cluster.
///
/// `display_name` and `colour` only make sense under `clusters.<id>`; at the top level they act
/// as a fallback for every cluster. Changes to `read_only`, `colour` and `display_name` reach
/// open sessions immediately; `exec_interactivity` is read when a session connects, so it takes
/// effect on the next connect; `default_namespace` is the namespace a session starts in, so it
/// does not move a selection the user already made.
///
/// Example, in `settings.json`:
///
/// ```json
/// "clusters": {
///   "3f2a9c1b7d4e8a60": {
///     "display_name": "Production (eu-west)",
///     "colour": "#e5484d",
///     "read_only": true,
///     "default_namespace": "payments",
///     "prometheus": { "url": "https://prometheus.example.com", "auth_secret": "prod-prometheus" }
///   }
/// }
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClusterSettingsContent {
    /// A name to show instead of the kubeconfig context name. Cluster ids are opaque hashes, so
    /// keep this next to the id to recognise the entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Accent colour of the cluster's tab, hotbar dot and badges, as `#rrggbb` or `#rgb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "colour_schema")]
    pub colour: Option<ClusterColour>,
    /// Block every mutation (create, edit, delete, scale, node shell) for the cluster, and shells and
    /// attaches into pods unless `exec_in_read_only` allows them.
    /// Set it under `clusters.<id>` to protect one cluster, or at the top level to start every
    /// cluster read-only and opt individual ones out with `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    /// The namespace a new session starts in. Unset uses the kubeconfig context's namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_namespace: Option<String>,
    /// Working directory of terminals opened for the cluster. Unset uses the home directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_cwd: Option<String>,
    /// Container image of the node shell pod; it needs `nsenter` and `sleep`. Unset uses the
    /// built-in image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_shell_image: Option<String>,
    /// Name of the image pull Secret (in the node shell's namespace) used to pull
    /// `node_shell_image` from a private registry. It names a Kubernetes Secret, it is not a
    /// credential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_shell_pull_secret: Option<String>,
    /// The rest of the node shell pod's template: namespace, command, `nsenter` options,
    /// tolerations, labels, image pull policy and lifetime. Fields merge across layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_shell: Option<NodeShellContent>,
    /// Manual Prometheus location; unset auto-detects. Fields merge across layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prometheus: Option<PrometheusContent>,
    /// Namespaces to offer in the selector when the user is not allowed to list namespaces
    /// cluster-wide (RBAC-restricted users). A list in a cluster replaces the one above it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessible_namespaces: Option<Vec<String>>,
    /// How interactive exec credential plugins (`aws`, `gke-gcloud-auth-plugin`, ...) may be:
    /// `never` (no stdin, never prompt; the GUI default), `if_available` or `always`. Read when a
    /// session connects: a change takes effect on the next connect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "exec_interactivity_schema")]
    pub exec_interactivity: Option<ExecInteractivity>,
    /// Allow a shell, attach or exec into a pod (`pod::Shell`, `pod::Attach`) while `read_only`
    /// is on. Off by default: a shell can change anything the container's user can, so a
    /// read-only cluster blocks it. Takes effect at once. Set it under `clusters.<id>` to let one
    /// read-only cluster keep its shells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_in_read_only: Option<bool>,
    /// Limits on the feeds the app opens on the cluster: feeds, objects, when to fall back to
    /// metadata-only feeds, and how long a feed nobody looks at keeps running. Fields merge
    /// across layers; a change applies to the next feed opened, without reconnecting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch_budget: Option<WatchBudgetContent>,
}
