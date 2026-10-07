//! Pure guard policy: the confirmation tier of a command, the cluster it acts on, and
//! what its audit record names as the target.

use oxikube_domain::command::{Command, CommandMeta, DEFAULT_DEBUG_IMAGE};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::safety::{ConfirmTier, Risk};

/// The confirmation tier the guard asks for before running a command.
///
/// * A non-mutating command never confirms ([`ConfirmTier::None`]).
/// * A mutating command takes the higher of its declared [`CommandMeta::confirm`] and
///   the tier its [`Risk`] maps to ([`Risk::confirm_tier`]), so metadata can raise the
///   friction but never lower it below the glossary table.
/// * A mutating command that declares no risk is treated as the worst case
///   ([`ConfirmTier::TypeName`]): the registry tests forbid it, the guard fails safe.
///
/// Target-sensitive raising lives in [`confirm_tier_for`] (what the guard uses); the per-session
/// "skip low-risk confirms" setting is E19.
pub fn confirm_tier(meta: &CommandMeta) -> ConfirmTier {
    if !meta.mutating {
        return ConfirmTier::None;
    }
    let from_risk = meta.risk.map_or(ConfirmTier::TypeName, Risk::confirm_tier);
    meta.confirm.max(from_risk)
}

/// The confirmation tier the guard asks for before running `command`: [`confirm_tier`] of its
/// metadata, raised by what the command targets. A `resource::Delete` of a Namespace, Node or
/// PersistentVolume, or with foreground propagation (a cascading delete), is
/// [`Command::effective_risk`] `High` or `Irreversible` and so takes a typed name (ADR 0012);
/// an ordinary object keeps the declared simple confirm. The tier never drops below the
/// declared one.
pub fn confirm_tier_for(meta: &CommandMeta, command: &Command) -> ConfirmTier {
    let raised = command
        .effective_risk()
        .map_or(ConfirmTier::None, Risk::confirm_tier);
    confirm_tier(meta).max(raised)
}

/// Whether `command` changes a cluster's safety posture (read-only mode, colour, preset).
///
/// Such a command never touches the cluster, so it is not `mutating` and the read-only check
/// does not apply to it (it must run on a read-only cluster, or nobody could turn the mode
/// off). It still goes through the guard's posture pipeline: confirmation when it lowers
/// protection on a production cluster, and an audit record.
pub fn is_posture(command: &Command) -> bool {
    matches!(
        command,
        Command::ClusterToggleReadOnly { .. }
            | Command::ClusterSetColour { .. }
            | Command::ClusterApplyPreset { .. }
    )
}

/// The cluster a command names in its payload, if any.
///
/// Resource verbs carry it in their [`ResourceRef`]; cluster verbs carry it directly.
/// UI-local commands (palette, zoom, windows) name none and act on
/// the active cluster from the dispatch context, if at all.
pub fn cluster_of(command: &Command) -> Option<&ClusterId> {
    match command {
        Command::ClusterConnect { cluster }
        | Command::ClusterCancelConnect { cluster }
        | Command::ClusterCloseTab { cluster }
        | Command::ClusterDisconnect { cluster }
        | Command::ClusterReconnect { cluster }
        | Command::ClusterSelect { cluster }
        | Command::ClusterToggleFavourite { cluster, .. }
        | Command::ClusterToggleReadOnly { cluster, .. }
        | Command::NamespaceSelect { cluster, .. }
        | Command::NamespaceToggleFavourite { cluster, .. }
        | Command::ClusterSetColour { cluster, .. }
        | Command::ClusterApplyPreset { cluster, .. }
        | Command::CrdOpenList { cluster }
        | Command::CrdOpenResources { cluster, .. }
        | Command::ResourceOpenList { cluster, .. }
        | Command::ResourceRetryFeed { cluster, .. }
        | Command::ResourceSelectAll { cluster, .. }
        | Command::TableFocusFilter { cluster, .. }
        | Command::ResourceApply { cluster, .. } => Some(cluster),
        Command::TerminalNew { cluster } => cluster.as_ref(),
        Command::ResourceOpen { target }
        | Command::ResourceCopyName { target }
        | Command::ResourcePinDetail { target }
        | Command::ResourceCopyLabel { target, .. }
        | Command::ResourceCopyYaml { target }
        | Command::ResourceSaveYaml { target }
        | Command::ResourceToggleManagedFields { target }
        | Command::ResourceRefreshDescribe { target }
        | Command::ResourceViewYaml { target }
        | Command::ResourceDelete { target, .. }
        | Command::PodDelete { target, .. }
        | Command::PodShell { target, .. }
        | Command::PodAttach { target, .. }
        | Command::PodExec { target, .. }
        | Command::PodDebug { target, .. }
        | Command::PodPortForward { target, .. }
        | Command::PodViewLogs { target, .. }
        | Command::LogsClear { target }
        | Command::LogsCopy { target }
        | Command::LogsMark { target }
        | Command::LogsSendToAgent { target }
        | Command::LogsTailInTerminal { target }
        | Command::LogsFollowReplacement { target }
        | Command::LogsReconnect { target }
        | Command::LogsSave { target, .. }
        | Command::LogsSetRange { target, .. }
        | Command::LogsSelectContainer { target, .. }
        | Command::LogsToggleAutoscroll { target }
        | Command::LogsToggleFullscreen { target }
        | Command::LogsTogglePrevious { target }
        | Command::LogsToggleTimestamps { target }
        | Command::LogsToggleSource { target, .. }
        | Command::LogsToggleWrap { target }
        | Command::LogsFind { target, .. }
        | Command::LogsNextMatch { target }
        | Command::LogsPreviousMatch { target }
        | Command::LogsToggleCase { target }
        | Command::LogsToggleInverse { target }
        | Command::LogsToggleFilterMode { target }
        | Command::LogsCloseSearch { target }
        | Command::LogsToggleJsonMode { target }
        | Command::LogsToggleLevel { target, .. }
        | Command::LogsToggleLine { target, .. }
        | Command::LogsCollapseLine { target }
        | Command::WorkloadScale { target, .. }
        | Command::WorkloadRestart { target }
        | Command::WorkloadViewLogs { target, .. }
        | Command::NodeCordon { target }
        | Command::NodeUncordon { target }
        | Command::NodeDrain { target, .. } => Some(&target.cluster),
        Command::ClusterNextTab
        | Command::ClusterPreviousTab
        | Command::ClusterSwitchTab { .. }
        | Command::KubeconfigAddSource { .. }
        | Command::KubeconfigRemoveSource { .. }
        | Command::KubeconfigReload
        | Command::ViewOpen { .. }
        | Command::PaletteToggle
        | Command::AppQuit
        | Command::WindowNew
        | Command::ViewZoomIn
        | Command::ViewZoomOut
        | Command::ViewZoomReset
        | Command::TerminalOpenLink { .. }
        | Command::TerminalCopy
        | Command::TerminalPaste
        | Command::TerminalSplit
        | Command::TerminalClose
        | Command::TerminalReconnect
        | Command::TerminalRestart
        | Command::TerminalClear
        | Command::TerminalScrollLineDown
        | Command::TerminalScrollLineUp
        | Command::TerminalScrollPageDown
        | Command::TerminalScrollPageUp
        | Command::TerminalSearch
        | Command::TerminalSearchClose
        | Command::TerminalSearchNext
        | Command::TerminalSearchPrevious
        | Command::TerminalSelectAll => None,
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
        let gvk = Gvk::new("oxikube.io", "v1", "Cluster");
        ResourceRef::cluster_scoped(cluster.clone(), gvk, "*")
    })
}

/// The one-line description the confirmation dialog shows, e.g.
/// `Delete Pod: Pod default/web-0 on kind-oxikube`. Names only, never payload fields.
pub fn summary(meta: &CommandMeta, command: &Command, context: &str) -> String {
    if let Command::PodDebug {
        target,
        image,
        target_container,
        ..
    } = command
    {
        return debug_summary(
            target,
            effective_image(image),
            target_container.as_deref(),
            context,
        );
    }
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

/// The confirmation text of `pod::Debug`: the pod, the image and the target container, and that the
/// container stays in the pod for good (an ephemeral container can be neither removed nor edited
/// until the pod is deleted).
fn debug_summary(
    pod: &ResourceRef,
    image: &str,
    target_container: Option<&str>,
    context: &str,
) -> String {
    let object = match pod.namespace() {
        Some(ns) => format!("{ns}/{}", pod.name),
        None => pod.name.to_string(),
    };
    let target = target_container.map_or_else(
        || "the pod's default container".to_owned(),
        |name| format!("container {name}"),
    );
    format!(
        "Add a debug container running {image} (sharing the processes of {target}) to Pod \
         {object} on {context}. It cannot be removed or edited afterwards: it stays in the pod \
         until the pod is deleted."
    )
}

/// The image a `pod::Debug` runs: the one named, else the default.
fn effective_image(image: &str) -> &str {
    let image = image.trim();
    if image.is_empty() {
        DEFAULT_DEBUG_IMAGE
    } else {
        image
    }
}

/// What the audit record of a guarded `command` says besides its target, if anything: for
/// `pod::Debug` the image, the target container, the program (never its arguments) and a chosen
/// name. Short, free of typed content, and redacted by the audit log like every field.
pub fn audit_detail(command: &Command) -> Option<String> {
    let Command::PodDebug {
        image,
        target_container,
        command,
        name,
        ..
    } = command
    else {
        return None;
    };
    let mut detail = format!(
        "session=debug image={} target={}",
        effective_image(image),
        target_container.as_deref().unwrap_or("(default)")
    );
    if let Some(program) = command.first() {
        detail.push_str(&format!(" program={program}"));
    }
    if let Some(name) = name {
        detail.push_str(&format!(" name={name}"));
    }
    Some(detail)
}

/// The text a [`ConfirmTier::TypeName`] confirmation must repeat: the target's name, or
/// the cluster's context name for a command without one.
pub fn expected_name(command: &Command, context: &str) -> String {
    command
        .target()
        .map_or_else(|| context.to_owned(), |t| t.name.to_string())
}
