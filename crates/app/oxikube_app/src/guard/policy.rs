//! Pure guard policy: the confirmation tier of a command, the cluster it acts on, and
//! what its audit record names as the target.

use oxikube_domain::command::{Command, CommandMeta};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::safety::{ConfirmTier, Risk};

/// The kind [`audit_target`] names when a command has no single target object.
const CLUSTER_KIND: (&str, &str, &str) = ("oxikube.io", "v1", "Cluster");

/// The confirmation tier the guard asks for before running a command.
///
/// * A non-mutating command never confirms ([`ConfirmTier::None`]).
/// * A mutating command takes the higher of its declared [`CommandMeta::confirm`] and
///   the tier its [`Risk`] maps to ([`Risk::confirm_tier`]), so metadata can raise the
///   friction but never lower it below the glossary table.
/// * A mutating command that declares no risk is treated as the worst case
///   ([`ConfirmTier::TypeName`]): the registry tests forbid it, the guard fails safe.
///
/// Target-sensitive raising (namespaces, nodes, PVs, cascading deletes) and the per-session
/// "skip low-risk confirms" setting are E19.
pub fn confirm_tier(meta: &CommandMeta) -> ConfirmTier {
    if !meta.mutating {
        return ConfirmTier::None;
    }
    let from_risk = meta.risk.map_or(ConfirmTier::TypeName, Risk::confirm_tier);
    meta.confirm.max(from_risk)
}

/// The cluster a command names in its payload, if any.
///
/// Resource verbs carry it in their [`ResourceRef`]; cluster verbs carry it directly.
/// UI-local commands (palette, zoom, windows, namespace selection) name none and act on
/// the active cluster from the dispatch context, if at all.
pub fn cluster_of(command: &Command) -> Option<&ClusterId> {
    match command {
        Command::ClusterSelect { cluster }
        | Command::ClusterToggleReadOnly { cluster, .. }
        | Command::ResourceOpenList { cluster, .. }
        | Command::ResourceApply { cluster, .. } => Some(cluster),
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
        | Command::NodeDrain { target, .. } => Some(&target.cluster),
        Command::NamespaceSelect { .. }
        | Command::ViewOpen { .. }
        | Command::PaletteToggle
        | Command::AppQuit
        | Command::WindowNew
        | Command::ViewZoomIn
        | Command::ViewZoomOut
        | Command::ViewZoomReset => None,
    }
}

/// What the audit record of `command` on `cluster` names as its target.
///
/// The command's own [`ResourceRef`] when it acts on one object. A command without a
/// single target (`resource::Apply` of a manifest) is recorded against the cluster
/// itself, as the synthetic cluster-scoped `oxikube.io/v1 Cluster` named `*`; the apply
/// service (E19) audits each applied object on its own.
pub fn audit_target(command: &Command, cluster: &ClusterId) -> ResourceRef {
    command.target().cloned().unwrap_or_else(|| {
        let (group, version, kind) = CLUSTER_KIND;
        ResourceRef::cluster_scoped(cluster.clone(), Gvk::new(group, version, kind), "*")
    })
}

/// The one-line description the confirmation dialog shows, e.g.
/// `Delete Pod: Pod default/web-0 on kind-oxikube`. Names only, never payload fields.
pub fn summary(meta: &CommandMeta, command: &Command, context: &str) -> String {
    match command.target() {
        Some(target) => {
            let object = match target.namespace() {
                Some(ns) => format!("{ns}/{}", target.name),
                None => target.name.to_string(),
            };
            format!("{}: {} {object} on {context}", meta.title, target.gvk.kind)
        }
        None => format!("{} on {context}", meta.title),
    }
}

/// The text a [`ConfirmTier::TypeName`] confirmation must repeat: the target's name, or
/// the cluster's context name for a command without one.
pub fn expected_name(command: &Command, context: &str) -> String {
    command
        .target()
        .map_or_else(|| context.to_owned(), |t| t.name.to_string())
}
