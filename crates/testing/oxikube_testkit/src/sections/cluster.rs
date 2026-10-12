//! Nodes (E12-S09, E12-S10), Namespaces (E12-S16) and the CustomResourceDefinition list
//! (E12-S17).

use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Scope;
use serde_json::{Value, json};

use super::{Api, KindCase, Seed};

/// Nodes: a section of one kind.
pub(super) const NODES: &[KindCase] = &[KindCase {
    section: "nodes",
    plural: "nodes",
    group: "",
    version: "v1",
    kind: "Node",
    scope: Scope::Cluster,
    api: Api::BuiltIn,
    columns: &[
        "Name",
        "Status",
        "Roles",
        "Version",
        "OS Image",
        "Kernel",
        "Internal IP",
        "Pods",
        "CPU",
        "CPU%",
        "Mem",
        "Mem%",
        "Taints",
        "Schedulable",
        "Age",
        "External IP*",
        "Instance Type*",
        "Zone*",
        "Container Runtime*",
        "Arch*",
        "Allocatable CPU*",
        "Allocatable Memory*",
        "Allocatable Pods*",
        "Labels*",
    ],
    actions: &[
        CommandId::NODE_SHELL,
        CommandId::NODE_CORDON,
        CommandId::NODE_UNCORDON,
        CommandId::NODE_DRAIN,
    ],
    panel: true,
    seed: node,
}];

/// Namespaces: a section of one kind.
pub(super) const NAMESPACES: &[KindCase] = &[KindCase {
    section: "namespaces",
    plural: "namespaces",
    group: "",
    version: "v1",
    kind: "Namespace",
    scope: Scope::Cluster,
    api: Api::BuiltIn,
    columns: &["Name", "Status", "Pods", "Age", "Labels*"],
    actions: &[
        CommandId::NAMESPACE_SELECT,
        CommandId::NAMESPACE_USE_FAVOURITE,
        CommandId::NAMESPACE_TOGGLE_FAVOURITE,
        CommandId::NAMESPACE_CREATE,
        CommandId::NAMESPACE_DELETE,
    ],
    panel: true,
    seed: |_| json!({"metadata": {}}),
}];

/// The Custom Resources section's "Definitions" entry: the CRD kind, which has no sidebar slot of
/// its own (`custom-resources/definitions` opens it).
pub(super) const DEFINITIONS: &[KindCase] = &[KindCase {
    section: "custom-resources",
    plural: "customresourcedefinitions",
    group: "apiextensions.k8s.io",
    version: "v1",
    kind: "CustomResourceDefinition",
    scope: Scope::Cluster,
    api: Api::BuiltIn,
    columns: &[
        "Name",
        "Kind",
        "Group",
        "Versions",
        "Scope",
        "Short Names",
        "Age",
        "Resource*",
        "Aliases*",
        "Established*",
    ],
    actions: &[CommandId::CRD_OPEN_RESOURCES, CommandId::RESOURCE_OPEN],
    panel: true,
    seed: definition,
}];

/// A Node no kubelet backs: only ever inserted into a fake cluster (the kind suite reads the
/// cluster's own node and never creates one).
fn node(_: &Seed<'_>) -> Value {
    json!({
        "metadata": {"labels": {"node-role.kubernetes.io/worker": ""}},
        "spec": {"unschedulable": true},
        "status": {
            "conditions": [{"type": "Ready", "status": "True"}],
            "nodeInfo": {"kubeletVersion": "v1.37.0", "osImage": "Debian", "kernelVersion": "6.1",
                         "containerRuntimeVersion": "containerd://2.0", "architecture": "arm64"},
            "addresses": [{"type": "InternalIP", "address": "10.0.0.9"}],
            "capacity": {"cpu": "4", "memory": "8Gi", "pods": "110"},
            "allocatable": {"cpu": "4", "memory": "8Gi", "pods": "110"},
        },
    })
}

/// A CRD of a group of the test's own (`<stem>.oxikube.test`), named [`definition_name`]. It
/// serves `Gadget`s nobody creates.
fn definition(seed: &Seed<'_>) -> Value {
    let group = format!("{}.oxikube.test", seed.stem);
    json!({
        "metadata": {},
        "spec": {
            "group": group,
            "scope": "Namespaced",
            "names": {"plural": "gadgets", "singular": "gadget", "kind": "Gadget",
                      "shortNames": ["gdg"]},
            "versions": [{
                "name": "v1", "served": true, "storage": true,
                "schema": {"openAPIV3Schema": {"type": "object",
                    "x-kubernetes-preserve-unknown-fields": true}},
            }],
        },
    })
}

/// The name a CRD must have: `<plural>.<group>` (a CRD's name is not free like other objects').
pub(super) fn definition_name(seed: &Seed<'_>) -> String {
    format!("gadgets.{}.oxikube.test", seed.stem)
}
