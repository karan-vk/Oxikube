//! The [`Command`] payload enum: navigation plus resource verbs.

use serde::{Deserialize, Serialize};

use super::id::CommandId;
use super::meta::CommandMeta;
use super::registry;
use crate::ids::{ClusterId, Gvk, ResourceRef};

/// How the API server deletes dependents of an object.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Propagation {
    /// Delete the object now; the garbage collector removes dependents later.
    #[default]
    Background,
    /// Delete dependents first, then the object.
    Foreground,
    /// Leave dependents in place.
    Orphan,
}

/// A user or agent action as plain data.
///
/// Serialised as JSON with a `type` tag equal to the command id
/// (`{"type":"workload::Scale","target":{..},"replicas":3}`), so an MCP tool
/// call or a keymap action with arguments converts to a `Command` with one
/// `serde_json::from_value`. Variants hold no closures and no UI types.
///
/// Each variant has exactly one [`CommandId`] ([`Command::id`]) and one
/// [`CommandMeta`] ([`Command::meta`]); see the module docs for the keymap /
/// command / tool mapping. Resource verbs act on a [`ResourceRef`], which
/// already carries the cluster, so they are valid regardless of which cluster
/// is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Command {
    /// Make a cluster the active one.
    #[serde(rename = "cluster::Select")]
    ClusterSelect {
        /// The cluster to activate.
        cluster: ClusterId,
    },
    /// Set (`Some`) or toggle (`None`) a cluster's read-only mode.
    #[serde(rename = "cluster::ToggleReadOnly")]
    ClusterToggleReadOnly {
        /// The cluster to change.
        cluster: ClusterId,
        /// Desired state; `None` flips the current one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        read_only: Option<bool>,
    },
    /// Choose the namespaces in view.
    #[serde(rename = "namespace::Select")]
    NamespaceSelect {
        /// Namespace names; empty means all namespaces.
        namespaces: Vec<String>,
    },
    /// Open a registered view (overview, events, ...) by id.
    #[serde(rename = "view::Open")]
    ViewOpen {
        /// View id as registered by the owning ui crate.
        view: String,
    },
    /// Show or hide the command palette.
    #[serde(rename = "palette::Toggle")]
    PaletteToggle,
    /// Open the list view of a resource kind.
    #[serde(rename = "resource::OpenList")]
    ResourceOpenList {
        /// Cluster to list in.
        cluster: ClusterId,
        /// The kind to list.
        gvk: Gvk,
    },
    /// Open a resource's detail view.
    #[serde(rename = "resource::Open")]
    ResourceOpen {
        /// The resource to open.
        target: ResourceRef,
    },
    /// Open a resource's YAML.
    #[serde(rename = "resource::ViewYaml")]
    ResourceViewYaml {
        /// The resource to show.
        target: ResourceRef,
    },
    /// Delete any resource.
    #[serde(rename = "resource::Delete")]
    ResourceDelete {
        /// The resource to delete.
        target: ResourceRef,
        /// Dependent handling.
        #[serde(default)]
        propagation: Propagation,
    },
    /// Apply a manifest (server-side apply after a dry-run diff).
    #[serde(rename = "resource::Apply")]
    ResourceApply {
        /// Cluster to apply to.
        cluster: ClusterId,
        /// Default namespace for namespaced objects without one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        /// The YAML or JSON manifest text. May hold secrets: audit and logs
        /// must redact it.
        manifest: String,
    },
    /// Delete one pod.
    #[serde(rename = "pod::Delete")]
    PodDelete {
        /// The pod to delete.
        target: ResourceRef,
        /// Grace period override in seconds; `None` uses the pod's own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grace_period_seconds: Option<u32>,
    },
    /// Run a command (or an interactive shell) in a container.
    #[serde(rename = "pod::Exec")]
    PodExec {
        /// The pod to exec into.
        target: ResourceRef,
        /// Container name; `None` picks the default container.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        container: Option<String>,
        /// Argv to run; empty means an interactive shell.
        #[serde(default)]
        command: Vec<String>,
    },
    /// Forward a local port to a pod port.
    #[serde(rename = "pod::PortForward")]
    PodPortForward {
        /// The pod to forward to.
        target: ResourceRef,
        /// Local port; `None` picks a free one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local_port: Option<u16>,
        /// The pod's port.
        remote_port: u16,
    },
    /// Open a pod's logs.
    #[serde(rename = "pod::ViewLogs")]
    PodViewLogs {
        /// The pod to read.
        target: ResourceRef,
        /// Container name; `None` means the default container.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        container: Option<String>,
        /// Keep streaming new lines.
        #[serde(default)]
        follow: bool,
        /// Show the previous container instance's logs.
        #[serde(default)]
        previous: bool,
        /// Only the last N lines.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tail_lines: Option<u32>,
    },
    /// Set a workload's replica count.
    #[serde(rename = "workload::Scale")]
    WorkloadScale {
        /// The Deployment, StatefulSet or ReplicaSet to scale.
        target: ResourceRef,
        /// Desired replicas.
        replicas: u32,
    },
    /// Rolling-restart a workload.
    #[serde(rename = "workload::Restart")]
    WorkloadRestart {
        /// The workload to restart.
        target: ResourceRef,
    },
    /// Mark a node unschedulable.
    #[serde(rename = "node::Cordon")]
    NodeCordon {
        /// The node.
        target: ResourceRef,
    },
    /// Mark a node schedulable again.
    #[serde(rename = "node::Uncordon")]
    NodeUncordon {
        /// The node.
        target: ResourceRef,
    },
    /// Cordon a node and evict its pods.
    #[serde(rename = "node::Drain")]
    NodeDrain {
        /// The node.
        target: ResourceRef,
        /// Also evict pods not managed by a controller.
        #[serde(default)]
        force: bool,
    },
}

impl Command {
    /// The command's id (keymap action name, serde `type` tag, tool name source).
    pub const fn id(&self) -> CommandId {
        match self {
            Command::ClusterSelect { .. } => CommandId::CLUSTER_SELECT,
            Command::ClusterToggleReadOnly { .. } => CommandId::CLUSTER_TOGGLE_READ_ONLY,
            Command::NamespaceSelect { .. } => CommandId::NAMESPACE_SELECT,
            Command::ViewOpen { .. } => CommandId::VIEW_OPEN,
            Command::PaletteToggle => CommandId::PALETTE_TOGGLE,
            Command::ResourceOpenList { .. } => CommandId::RESOURCE_OPEN_LIST,
            Command::ResourceOpen { .. } => CommandId::RESOURCE_OPEN,
            Command::ResourceViewYaml { .. } => CommandId::RESOURCE_VIEW_YAML,
            Command::ResourceDelete { .. } => CommandId::RESOURCE_DELETE,
            Command::ResourceApply { .. } => CommandId::RESOURCE_APPLY,
            Command::PodDelete { .. } => CommandId::POD_DELETE,
            Command::PodExec { .. } => CommandId::POD_EXEC,
            Command::PodPortForward { .. } => CommandId::POD_PORT_FORWARD,
            Command::PodViewLogs { .. } => CommandId::POD_VIEW_LOGS,
            Command::WorkloadScale { .. } => CommandId::WORKLOAD_SCALE,
            Command::WorkloadRestart { .. } => CommandId::WORKLOAD_RESTART,
            Command::NodeCordon { .. } => CommandId::NODE_CORDON,
            Command::NodeUncordon { .. } => CommandId::NODE_UNCORDON,
            Command::NodeDrain { .. } => CommandId::NODE_DRAIN,
        }
    }

    /// The command's static metadata.
    ///
    /// # Panics
    ///
    /// Panics if the registry lacks this variant's id; the registry tests make
    /// that impossible in a green build.
    pub fn meta(&self) -> &'static CommandMeta {
        registry::lookup(self.id()).expect("every Command variant has a registry entry")
    }

    /// Whether this command goes through `MutationGuard`.
    pub fn is_mutating(&self) -> bool {
        self.meta().mutating
    }

    /// The resource this command targets, if it acts on a single selection.
    pub fn target(&self) -> Option<&ResourceRef> {
        match self {
            Command::ResourceOpen { target }
            | Command::ResourceViewYaml { target }
            | Command::ResourceDelete { target, .. }
            | Command::PodDelete { target, .. }
            | Command::PodExec { target, .. }
            | Command::PodPortForward { target, .. }
            | Command::PodViewLogs { target, .. }
            | Command::WorkloadScale { target, .. }
            | Command::WorkloadRestart { target }
            | Command::NodeCordon { target }
            | Command::NodeUncordon { target }
            | Command::NodeDrain { target, .. } => Some(target),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;
    use crate::command::COMMANDS;
    use crate::ids::ContextName;

    fn cluster() -> ClusterId {
        ClusterId::new("~/.kube/config", &ContextName::new("kind-oxikube"))
    }

    fn pod() -> ResourceRef {
        ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "default", "web-0")
    }

    fn deployment() -> ResourceRef {
        ResourceRef::namespaced(
            cluster(),
            Gvk::new("apps", "v1", "Deployment"),
            "prod",
            "api",
        )
    }

    fn node() -> ResourceRef {
        ResourceRef::cluster_scoped(cluster(), Gvk::new("", "v1", "Node"), "worker-1")
    }

    /// One sample of every variant.
    fn samples() -> Vec<Command> {
        vec![
            Command::ClusterSelect { cluster: cluster() },
            Command::ClusterToggleReadOnly {
                cluster: cluster(),
                read_only: None,
            },
            Command::ClusterToggleReadOnly {
                cluster: cluster(),
                read_only: Some(true),
            },
            Command::NamespaceSelect {
                namespaces: vec!["default".into(), "kube-system".into()],
            },
            Command::ViewOpen {
                view: "overview".into(),
            },
            Command::PaletteToggle,
            Command::ResourceOpenList {
                cluster: cluster(),
                gvk: Gvk::new("apps", "v1", "Deployment"),
            },
            Command::ResourceOpen { target: pod() },
            Command::ResourceViewYaml { target: pod() },
            Command::ResourceDelete {
                target: deployment(),
                propagation: Propagation::Foreground,
            },
            Command::ResourceApply {
                cluster: cluster(),
                namespace: Some("default".into()),
                manifest: "kind: ConfigMap".into(),
            },
            Command::PodDelete {
                target: pod(),
                grace_period_seconds: Some(0),
            },
            Command::PodExec {
                target: pod(),
                container: Some("app".into()),
                command: vec!["sh".into(), "-c".into(), "id".into()],
            },
            Command::PodPortForward {
                target: pod(),
                local_port: None,
                remote_port: 8080,
            },
            Command::PodViewLogs {
                target: pod(),
                container: None,
                follow: true,
                previous: false,
                tail_lines: Some(500),
            },
            Command::WorkloadScale {
                target: deployment(),
                replicas: 3,
            },
            Command::WorkloadRestart {
                target: deployment(),
            },
            Command::NodeCordon { target: node() },
            Command::NodeUncordon { target: node() },
            Command::NodeDrain {
                target: node(),
                force: true,
            },
        ]
    }

    #[test]
    fn json_round_trip_for_every_variant() {
        for command in samples() {
            let json = serde_json::to_value(&command).unwrap();
            assert_eq!(
                json["type"],
                command.id().as_str(),
                "type tag must equal the command id"
            );
            let back: Command = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(back, command, "{json}");
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
        }
    }

    #[test]
    fn samples_cover_exactly_the_registry() {
        let from_samples: BTreeSet<_> = samples().iter().map(Command::id).collect();
        let from_registry: BTreeSet<_> = COMMANDS.iter().map(|m| m.id).collect();
        assert_eq!(from_samples, from_registry);
        for command in samples() {
            assert_eq!(command.meta().id, command.id());
        }
    }

    #[test]
    fn tool_call_arguments_become_a_command() {
        // An MCP `k8s.workload_scale` call: the tool name picks the id, the
        // arguments are the payload.
        let args = json!({ "target": deployment(), "replicas": 5 });
        let id = COMMANDS
            .iter()
            .find(|m| m.id.tool_name() == "k8s.workload_scale")
            .unwrap()
            .id;
        let mut value = args;
        value["type"] = json!(id.as_str());
        let command: Command = serde_json::from_value(value).unwrap();
        assert_eq!(
            command,
            Command::WorkloadScale {
                target: deployment(),
                replicas: 5
            }
        );
        assert!(command.is_mutating());
        assert_eq!(command.target(), Some(&deployment()));
    }

    #[test]
    fn optional_fields_default() {
        let delete: Command = serde_json::from_value(json!({
            "type": "resource::Delete",
            "target": deployment(),
        }))
        .unwrap();
        assert_eq!(
            delete,
            Command::ResourceDelete {
                target: deployment(),
                propagation: Propagation::Background
            }
        );
        let logs: Command = serde_json::from_value(json!({
            "type": "pod::ViewLogs",
            "target": pod(),
        }))
        .unwrap();
        assert!(matches!(
            logs,
            Command::PodViewLogs {
                follow: false,
                previous: false,
                container: None,
                tail_lines: None,
                ..
            }
        ));
        let unit: Command = serde_json::from_value(json!({ "type": "palette::Toggle" })).unwrap();
        assert_eq!(unit, Command::PaletteToggle);
    }

    #[test]
    fn rejects_unknown_or_missing_type() {
        assert!(serde_json::from_value::<Command>(json!({ "type": "pod::Explode" })).is_err());
        assert!(serde_json::from_value::<Command>(json!({ "replicas": 3 })).is_err());
        assert!(
            serde_json::from_value::<Command>(
                json!({ "type": "workload::Scale", "target": pod() })
            )
            .is_err()
        );
    }

    #[test]
    fn navigation_commands_do_not_mutate() {
        for command in samples() {
            if matches!(
                command,
                Command::ClusterSelect { .. }
                    | Command::NamespaceSelect { .. }
                    | Command::ViewOpen { .. }
                    | Command::PaletteToggle
                    | Command::ResourceOpen { .. }
                    | Command::ResourceOpenList { .. }
                    | Command::ResourceViewYaml { .. }
                    | Command::ClusterToggleReadOnly { .. }
            ) {
                assert!(!command.is_mutating(), "{}", command.id());
            }
        }
    }
}
