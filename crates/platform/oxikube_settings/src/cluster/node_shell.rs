//! The `node_shell` block (E09-S09): the template of the privileged pod a node shell runs in.
//!
//! `node_shell_image` and `node_shell_pull_secret` stay flat keys beside it (E06-S08); everything
//! newer lives in this block, so one `node_shell` object in a cluster's block overrides fields of
//! the user's top-level one, which overrides the defaults in `default.json`.

use std::collections::BTreeMap;

use oxikube_ports::{NodeShellPrefs, NodeShellToleration};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::resolved::non_blank;

/// How a toleration matches a taint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TolerationOperator {
    /// Any value of the key (or every key when `key` is unset).
    Exists,
    /// The key with exactly `value`.
    Equal,
}

/// Which taint effect a toleration covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TaintEffect {
    /// New pods are not scheduled onto the node.
    NoSchedule,
    /// The scheduler avoids the node.
    PreferNoSchedule,
    /// Running pods are evicted.
    NoExecute,
}

/// When the node shell image is pulled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ImagePullPolicy {
    /// Always pull.
    Always,
    /// Pull when the node does not have the image.
    IfNotPresent,
    /// Never pull: the node must have the image.
    Never,
}

/// One toleration of the node shell pod (a Kubernetes `Toleration`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TolerationContent {
    /// The taint key; unset with `operator: Exists` matches every key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// `Exists` or `Equal` (Kubernetes' own default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<TolerationOperator>,
    /// The taint value, for `Equal`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The taint effect; unset covers every effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<TaintEffect>,
    /// For `NoExecute`: how long the pod stays after the taint appears.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toleration_seconds: Option<i64>,
}

/// The node shell pod template, as one settings layer writes it. Every field is optional so a
/// layer sets any subset.
///
/// Example, to run on a tainted GPU node from a mirror registry:
///
/// ```json
/// "node_shell_image": "registry.example/tools/nsenter:1",
/// "node_shell": {
///   "namespace": "ops-debug",
///   "tolerations": [{ "key": "nvidia.com/gpu", "operator": "Exists" }],
///   "labels": { "team": "infra" }
/// }
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NodeShellContent {
    /// Namespace of the helper pod. The default, `kube-system`, exists everywhere and is exempt
    /// from the pod security admission that refuses privileged pods in ordinary namespaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// The command run in the node's namespaces, after the `nsenter` options. Empty runs
    /// `bash -l` when the node has bash, else `sh -l`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// The `nsenter` options before the `--`. Empty enters every namespace of the node's init
    /// process: `-t 1 -m -u -i -n -p`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nsenter_args: Option<Vec<String>>,
    /// What the pod tolerates. The default tolerates every taint; an empty list tolerates
    /// none. A list replaces the one of the layer above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerations: Option<Vec<TolerationContent>>,
    /// Labels added to the pod. Oxikube's own labels cannot be replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<BTreeMap<String, String>>,
    /// `Always`, `IfNotPresent` or `Never`; unset leaves the cluster's default for the tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_pull_policy: Option<ImagePullPolicy>,
    /// The longest the pod lives, in seconds (`activeDeadlineSeconds`): the cluster deletes a
    /// pod Oxikube could not. Unset is eight hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lifetime_seconds: Option<u64>,
}

impl From<TolerationContent> for NodeShellToleration {
    fn from(content: TolerationContent) -> Self {
        let name = |value: Option<&str>| value.map(str::to_owned);
        Self {
            key: non_blank(content.key),
            operator: name(content.operator.map(|op| match op {
                TolerationOperator::Exists => "Exists",
                TolerationOperator::Equal => "Equal",
            })),
            value: content.value,
            effect: name(content.effect.map(|effect| match effect {
                TaintEffect::NoSchedule => "NoSchedule",
                TaintEffect::PreferNoSchedule => "PreferNoSchedule",
                TaintEffect::NoExecute => "NoExecute",
            })),
            toleration_seconds: content.toleration_seconds,
        }
    }
}

impl From<NodeShellContent> for NodeShellPrefs {
    fn from(content: NodeShellContent) -> Self {
        Self {
            namespace: non_blank(content.namespace),
            command: content.command.unwrap_or_default(),
            nsenter_args: content.nsenter_args.unwrap_or_default(),
            tolerations: content
                .tolerations
                .map(|list| list.into_iter().map(Into::into).collect()),
            labels: content.labels.unwrap_or_default(),
            image_pull_policy: content.image_pull_policy.map(|policy| {
                match policy {
                    ImagePullPolicy::Always => "Always",
                    ImagePullPolicy::IfNotPresent => "IfNotPresent",
                    ImagePullPolicy::Never => "Never",
                }
                .to_owned()
            }),
            max_lifetime_seconds: content.max_lifetime_seconds.filter(|seconds| *seconds > 0),
        }
    }
}
