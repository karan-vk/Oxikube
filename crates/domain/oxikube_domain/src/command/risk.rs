//! Target-sensitive risk: how dangerous one command is for the object it names.
//!
//! A [`CommandMeta`](super::CommandMeta) declares one risk per command, which is the floor. Some
//! commands are far more dangerous for some targets than for others (deleting a Pod is routine,
//! deleting a Namespace removes everything in it), so the guard asks [`Command::effective_risk`]
//! and takes the higher of the two. The rule is pure data and lives here so the guard, the
//! confirmation dialog and the MCP tool stubs all read one table (ADR 0012).

use super::{Command, CommandId, CommandMeta, Propagation};
use crate::ids::Gvk;
use crate::safety::Risk;

/// The risk of deleting one object of kind `gvk` with `propagation`.
///
/// | Target | Risk | Confirmation |
/// |---|---|---|
/// | `Namespace`, `PersistentVolume` | [`Risk::Irreversible`] | type the name |
/// | `Node`, or any object with [`Propagation::Foreground`] (a cascading delete) | [`Risk::High`] | type the name |
/// | anything else | [`Risk::Medium`] | simple confirm |
///
/// The kind is matched on group and kind only (not version), so `v1` and any future version of
/// the core kinds match, and a custom resource that happens to be named `Node` in another group
/// does not.
pub fn delete_risk(gvk: &Gvk, propagation: Propagation) -> Risk {
    let core = gvk.group.is_empty();
    match &*gvk.kind {
        "Namespace" | "PersistentVolume" if core => Risk::Irreversible,
        "Node" if core => Risk::High,
        _ if propagation == Propagation::Foreground => Risk::High,
        _ => Risk::Medium,
    }
}

impl Command {
    /// The risk of this command for the object it names: the declared [`CommandMeta::risk`]
    /// raised by the target ([`delete_risk`] for `resource::Delete`). `None` for a command that
    /// is not mutating.
    ///
    /// [`CommandMeta::risk`]: super::CommandMeta::risk
    pub fn effective_risk(&self) -> Option<Risk> {
        let declared = self.meta().risk?;
        Some(match self {
            Command::ResourceDelete {
                target,
                propagation,
            } => declared.max(delete_risk(&target.gvk, *propagation)),
            _ => declared,
        })
    }
}

impl CommandMeta {
    /// The risk the MCP tool stub advertises: the worst case over every target the command can
    /// name. A tool is one static entry that cannot vary by target, so it must not understate
    /// what `resource.delete` does to a Namespace or PersistentVolume; for every other command
    /// it is the declared [`CommandMeta::risk`]. An exec-class command ([`CommandMeta::exec`])
    /// declares no risk (it changes no object), but a shell can do anything the container's user
    /// can, so its tool stub advertises [`Risk::High`]. The guard still decides per target
    /// ([`Command::effective_risk`]).
    pub fn tool_risk(&self) -> Option<Risk> {
        if self.exec {
            return Some(Risk::High);
        }
        let declared = self.risk?;
        Some(if self.id == CommandId::RESOURCE_DELETE {
            declared.max(Risk::Irreversible)
        } else {
            declared
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ClusterId, ContextName, ResourceRef};

    fn gvk(group: &str, kind: &str) -> Gvk {
        Gvk::new(group, "v1", kind)
    }

    #[test]
    fn ordinary_objects_are_medium() {
        for (group, kind) in [
            ("", "Pod"),
            ("apps", "Deployment"),
            ("", "ConfigMap"),
            ("example.io", "Widget"),
        ] {
            assert_eq!(
                delete_risk(&gvk(group, kind), Propagation::Background),
                Risk::Medium,
                "{kind}"
            );
            assert_eq!(
                delete_risk(&gvk(group, kind), Propagation::Orphan),
                Risk::Medium,
                "orphaning leaves dependents alone: {kind}"
            );
        }
    }

    #[test]
    fn namespaces_nodes_and_volumes_are_raised() {
        assert_eq!(
            delete_risk(&gvk("", "Namespace"), Propagation::Background),
            Risk::Irreversible
        );
        assert_eq!(
            delete_risk(&gvk("", "PersistentVolume"), Propagation::Background),
            Risk::Irreversible
        );
        assert_eq!(
            delete_risk(&gvk("", "Node"), Propagation::Background),
            Risk::High
        );
    }

    #[test]
    fn a_namesake_in_another_group_is_ordinary() {
        assert_eq!(
            delete_risk(&gvk("example.io", "Node"), Propagation::Background),
            Risk::Medium
        );
    }

    #[test]
    fn a_cascading_delete_is_high() {
        assert_eq!(
            delete_risk(&gvk("apps", "Deployment"), Propagation::Foreground),
            Risk::High
        );
        // The higher of the two applies.
        assert_eq!(
            delete_risk(&gvk("", "Namespace"), Propagation::Foreground),
            Risk::Irreversible
        );
    }

    #[test]
    fn effective_risk_is_at_least_the_declared_one() {
        let cluster = ClusterId::new("~/.kube/config", &ContextName::new("kind"));
        let delete = |gvk: Gvk, propagation| Command::ResourceDelete {
            target: ResourceRef::cluster_scoped(cluster.clone(), gvk, "x"),
            propagation,
        };
        let pod = delete(gvk("", "Pod"), Propagation::Background);
        assert_eq!(pod.effective_risk(), pod.meta().risk);
        let namespace = delete(gvk("", "Namespace"), Propagation::Background);
        assert_eq!(namespace.effective_risk(), Some(Risk::Irreversible));
        let read = Command::ResourceOpen {
            target: ResourceRef::cluster_scoped(cluster, gvk("", "Pod"), "x"),
        };
        assert_eq!(read.effective_risk(), None);
    }

    #[test]
    fn the_delete_tool_advertises_the_worst_case_target() {
        let meta = super::super::lookup(CommandId::RESOURCE_DELETE).unwrap();
        assert_eq!(meta.risk, Some(Risk::Medium));
        assert_eq!(meta.tool_risk(), Some(Risk::Irreversible));
        let pod_logs = super::super::lookup(CommandId::POD_VIEW_LOGS).unwrap();
        assert_eq!(pod_logs.tool_risk(), None);
        let apply = super::super::lookup(CommandId::RESOURCE_APPLY).unwrap();
        assert_eq!(apply.tool_risk(), apply.risk);
    }
}
