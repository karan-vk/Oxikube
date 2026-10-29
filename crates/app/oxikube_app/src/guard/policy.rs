//! Pure guard policy: the confirmation tier of a command, the cluster it acts on, and
//! what its audit record names as the target.

use oxikube_domain::command::{Command, CommandId, CommandMeta, DEFAULT_DEBUG_IMAGE};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_ports::{ClusterPrefs, NodeShellSpec};

use crate::actions::object_label;
use crate::workloads::replicas_word;

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
    is_posture_id(command.id())
}

/// Whether the command `id` names is a posture command ([`is_posture`]).
pub fn is_posture_id(id: CommandId) -> bool {
    [
        CommandId::CLUSTER_TOGGLE_READ_ONLY,
        CommandId::CLUSTER_SET_COLOUR,
        CommandId::CLUSTER_APPLY_PRESET,
    ]
    .contains(&id)
}

/// The cluster a command names in its payload, if any.
///
/// Resource verbs carry it in their [`ResourceRef`]; cluster verbs carry it directly.
/// UI-local commands (palette, zoom, windows) name none and act on
/// the active cluster from the dispatch context, if at all.
pub fn cluster_of(command: &Command) -> Option<&ClusterId> {
    match command {
        // A bulk command acts on one cluster: the first object's (the handler refuses a mix).
        Command::SelectionDelete { targets, .. }
        | Command::SelectionRestart { targets }
        | Command::SelectionLabel { targets, .. } => targets.first().map(|target| &target.cluster),
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
        | Command::NamespaceUseFavourite { cluster, .. }
        | Command::NamespaceCreate { cluster, .. }
        | Command::ClusterSetColour { cluster, .. }
        | Command::ClusterShowMetricsSettings { cluster }
        | Command::ClusterShowPulses { cluster }
        | Command::HelmOpenReleases { cluster }
        | Command::RbacOpenUsers { cluster }
        | Command::RbacOpenGroups { cluster }
        | Command::RbacShowSubject { cluster, .. }
        | Command::RbacCopyMatrix { cluster }
        | Command::HelmShowValues { cluster, .. }
        | Command::HelmShowManifest { cluster, .. }
        | Command::HelmRevealValues { cluster, .. }
        | Command::RbacCanI { cluster, .. }
        | Command::EventsOpen { cluster, .. }
        | Command::EventsToggleFaults { cluster }
        | Command::EventsTogglePause { cluster }
        | Command::EventsCopyRow { cluster }
        | Command::EventsCopyRowJson { cluster }
        | Command::EventsExportJson { cluster }
        | Command::NotificationsMuteCluster { cluster }
        | Command::NotificationsUnmuteCluster { cluster }
        | Command::ClusterShowOverview { cluster }
        | Command::EventsSendToAgent { cluster, .. }
        | Command::ClusterRedetectMetrics { cluster }
        | Command::ClusterToggleMetrics { cluster, .. }
        | Command::ClusterApplyPreset { cluster, .. }
        | Command::CrdOpenList { cluster }
        | Command::CrdOpenResources { cluster, .. }
        | Command::ResourceOpenList { cluster, .. }
        | Command::ResourceRetryFeed { cluster, .. }
        | Command::ResourceSelectAll { cluster, .. }
        | Command::TableFocusFilter { cluster, .. }
        | Command::TableToggleWide { cluster, .. }
        | Command::TableSetFilter { cluster, .. }
        | Command::PodSanitize { cluster, .. }
        | Command::HelmInstall { cluster, .. }
        | Command::HelmUpgrade { cluster, .. }
        | Command::HelmRollback { cluster, .. }
        | Command::HelmUninstall { cluster, .. }
        | Command::RoleCreate { cluster, .. }
        | Command::RoleEdit { cluster, .. }
        | Command::BindingCreate { cluster, .. }
        | Command::BindingEdit { cluster, .. }
        | Command::ResourceApply { cluster, .. } => Some(cluster),
        Command::RoleOpenDialog {
            cluster, target, ..
        }
        | Command::BindingOpenDialog {
            cluster, target, ..
        } => cluster
            .as_ref()
            .or_else(|| target.as_ref().map(|target| &target.cluster)),
        Command::TerminalNew { cluster }
        | Command::EditorNewManifest { cluster }
        | Command::SecretCreate { cluster }
        | Command::ResourceCreate { cluster, .. } => cluster.as_ref(),
        Command::ResourceOpen { target }
        | Command::ResourceCopyName { target }
        | Command::ResourcePinDetail { target }
        | Command::ResourceCopyLabel { target, .. }
        | Command::ResourceCopyYaml { target }
        | Command::ResourceSaveYaml { target }
        | Command::ResourceToggleManagedFields { target }
        | Command::ResourceFind { target, .. }
        | Command::ResourceNextMatch { target }
        | Command::ResourcePreviousMatch { target }
        | Command::ResourceRefreshDescribe { target }
        | Command::ResourceViewYaml { target }
        | Command::ResourceViewYamlInEditor { target }
        | Command::ResourceViewDescribe { target }
        | Command::ResourceViewEvents { target }
        | Command::ResourceViewMetrics { target }
        | Command::ResourceEdit { target }
        | Command::SecretReveal { target, .. }
        | Command::SecretEditDecoded { target }
        | Command::ResourceDelete { target, .. }
        | Command::NamespaceDelete { target, .. }
        | Command::PodDelete { target, .. }
        | Command::PodEvict { target }
        | Command::PodForceDelete { target }
        | Command::PodJumpToOwner { target }
        | Command::PodShowNode { target }
        | Command::PodShell { target, .. }
        | Command::PodAttach { target, .. }
        | Command::PodExec { target, .. }
        | Command::PodDebug { target, .. }
        | Command::PodPortForward { target, .. }
        | Command::PortForwardStart { target, .. }
        | Command::PortForwardFavouriteAdd { target, .. }
        | Command::ServiceOpenInBrowser { target, .. }
        | Command::ServiceShowPods { target }
        | Command::ResourceShowUsedBy { target }
        | Command::ResourceDiffWith { target, .. }
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
        | Command::WorkloadSetImage { target, .. }
        | Command::WorkloadRollback { target, .. }
        | Command::WorkloadViewLogs { target, .. }
        | Command::NodeCordon { target }
        | Command::NodeShell { target }
        | Command::NodeUncordon { target }
        | Command::IngressClassSetDefault { target, .. }
        | Command::NodeDrain { target, .. }
        | Command::CronJobTrigger { target }
        | Command::CronJobSuspend { target }
        | Command::CronJobResume { target }
        | Command::JobSuspend { target }
        | Command::JobResume { target }
        | Command::PvcResize { target, .. }
        | Command::RoleDelete { target, .. }
        | Command::ServiceAccountGenerateKubeconfig { target, .. }
        | Command::BindingDelete { target }
        | Command::StorageClassSetDefault { target, .. } => Some(&target.cluster),
        Command::ClusterNextTab
        | Command::ClusterPreviousTab
        | Command::ClusterSwitchTab { .. }
        | Command::KubeconfigAddSource { .. }
        | Command::KubeconfigRemoveSource { .. }
        | Command::KubeconfigReload
        | Command::CloudRefresh { .. }
        | Command::CloudCancel { .. }
        | Command::CloudAdd { .. }
        | Command::CloudRemove { .. }
        | Command::ViewOpen { .. }
        | Command::PaletteToggle
        | Command::PaletteClearRecents
        | Command::PaletteOpenJump
        | Command::HelpShow
        | Command::PaletteToggleShowAll
        | Command::KeymapOpenUser
        | Command::ChartsOpenGallery
        | Command::AppQuit
        | Command::AppSendTestNotification
        | Command::AppToggleOsNotifications { .. }
        | Command::HelmRecheck
        | Command::HelmLocateBinary
        | Command::NotificationsTogglePanel
        | Command::NotificationsMarkRead { .. }
        | Command::NotificationsMarkAllRead
        | Command::PortForwardStop { .. }
        | Command::PortForwardRestart { .. }
        | Command::PortForwardRemove { .. }
        | Command::PortForwardFavouriteRemove { .. }
        | Command::PortForwardFavouriteToggleAutoStart { .. }
        | Command::PortForwardCopyUrl { .. }
        | Command::PortForwardOpenInBrowser { .. }
        | Command::PortForwardTogglePanel { .. }
        | Command::HelmShowRepos
        | Command::HelmOpenCharts
        | Command::HelmSearchCharts { .. }
        | Command::HelmShowChart { .. }
        | Command::HelmInstallChart { .. }
        | Command::HelmUpgradeRelease { .. }
        | Command::HelmRepoAdd { .. }
        | Command::HelmRepoRemove { .. }
        | Command::HelmRepoUpdate { .. }
        | Command::NotificationsOpenLink { .. }
        | Command::JumpBack
        | Command::JumpForward
        | Command::JumpLast
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
        | Command::TerminalSelectAll
        | Command::EditorToggleManagedFields
        | Command::EditorToggleSecretMask
        | Command::EditorToggleReadOnly
        | Command::EditorToggleSoftWrap
        | Command::EditorConvertToJson
        | Command::EditorConvertToYaml
        | Command::EditorUseJson
        | Command::EditorUseYaml
        | Command::EditorTriggerCompletion
        | Command::EditorShowHover => None,
        Command::EditorShowDiff
        | Command::EditorToggleDiffLayout
        | Command::EditorReloadFromServer
        | Command::EditorMergeFromServer
        | Command::EditorKeepMine
        | Command::EditorTakeMine { .. }
        | Command::EditorTakeServer { .. }
        | Command::EditorAcceptMerge => None,
    }
}

/// What the audit record of `command` on `cluster` names as its target.
///
/// The command's own [`ResourceRef`] when it acts on one object, and for `resource::Apply` the
/// object its manifest names (E10-S08). A command without a single target (a manifest that
/// names no single object) is recorded against the cluster itself, as the synthetic
/// cluster-scoped `oxikube.io/v1 Cluster` named `*`.
pub fn audit_target(command: &Command, cluster: &ClusterId) -> ResourceRef {
    if let Some(object) = crate::apply::audit_target(command, cluster) {
        return object;
    }
    if let Some(release) = crate::helm_install::audit_target(command, cluster) {
        return release;
    }
    if let Some(release) = crate::helm_upgrade::audit_target(command, cluster) {
        return release;
    }
    if let Some(release) = crate::helm_ops::audit_target(command, cluster) {
        return release;
    }
    if let Some(role) = crate::role_ops::audit_target(command, cluster) {
        return role;
    }
    if let Some(binding) = crate::binding_ops::audit_target(command, cluster) {
        return binding;
    }
    // A create names the object it will make.
    if let Command::NamespaceCreate { name, .. } = command {
        return ResourceRef::cluster_scoped(cluster.clone(), Gvk::namespace(), &**name);
    }
    // A sweep of one namespace is recorded against that namespace.
    if let Command::PodSanitize {
        namespace: Some(namespace),
        ..
    } = command
    {
        return ResourceRef::cluster_scoped(cluster.clone(), Gvk::namespace(), namespace.as_str());
    }
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
    if let Some(summary) = super::workload_policy::summary(command, context) {
        return summary;
    }
    if let Some(summary) = super::selection_policy::summary(command, context) {
        return summary;
    }
    if let Some(summary) = crate::helm_install::summary(command, context) {
        return summary;
    }
    if let Some(summary) = crate::helm_upgrade::summary(command, context) {
        return summary;
    }
    if let Some(summary) = crate::helm_ops::summary(command, context) {
        return summary;
    }
    if let Some(summary) = crate::role_ops::summary(command, context) {
        return summary;
    }
    if let Some(summary) = crate::sa_kubeconfig::summary(command, context) {
        return summary;
    }
    if let Some(summary) = crate::binding_ops::summary(command, context) {
        return summary;
    }
    if let Command::WorkloadScale { target, replicas } = command {
        return scale_summary(target, *replicas, context);
    }
    if let Command::WorkloadRollback {
        target,
        to_revision,
    } = command
    {
        return rollback_summary(target, *to_revision, context);
    }
    if let Command::PvcResize { target, size, from } = command {
        let object = match target.namespace() {
            Some(ns) => format!("{ns}/{}", target.name),
            None => target.name.to_string(),
        };
        let from = from
            .as_deref()
            .map_or_else(String::new, |f| format!(" from {f}"));
        return format!("Resize PersistentVolumeClaim {object}{from} to {size} on {context}");
    }
    let base = match command.target() {
        Some(target) => {
            let object = match target.namespace() {
                Some(ns) => format!("{ns}/{}", target.name),
                None => target.name.to_string(),
            };
            format!("{}: {} {object} on {context}", meta.title, target.gvk.kind)
        }
        None => format!("{} on {context}", meta.title),
    };
    match command {
        // Name the second write, so the user confirms what else changes.
        Command::IngressClassSetDefault {
            clear_previous: true,
            ..
        } => format!("{base}. Clears the default flag of the other Ingress classes."),
        _ => base,
    }
}

/// The confirmation text of `workload::Scale`: the workload, the count, and for zero that every
/// pod stops.
fn scale_summary(workload: &ResourceRef, replicas: u32, context: &str) -> String {
    let zero = if replicas == 0 {
        " (stops all of its pods)"
    } else {
        ""
    };
    format!(
        "Scale {} to {replicas} {}{zero} on {context}",
        object_label(workload),
        replicas_word(replicas)
    )
}

/// The confirmation text of `workload::Rollback`: the workload, the revision, and that the pods
/// are replaced by a rolling update.
fn rollback_summary(workload: &ResourceRef, to_revision: Option<i64>, context: &str) -> String {
    // A ReplicaSet has no history of its own: its Deployment goes back to its revision.
    let (subject, default_revision) = if &*workload.gvk.kind == "ReplicaSet" {
        (
            format!("the Deployment that owns {}", object_label(workload)),
            "this ReplicaSet's revision",
        )
    } else {
        (object_label(workload), "the previous revision")
    };
    let to = to_revision.map_or_else(
        || default_revision.to_owned(),
        |revision| format!("revision {revision}"),
    );
    format!(
        "Roll back {subject} to {to} on {context}: its pod template is replaced and its pods are \
         replaced by a rolling update."
    )
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
/// name; for `workload::Scale` the requested count and for `workload::SetImage` the containers
/// named; for `resource::Apply` the field manager, force and the reviewed version (never the
/// manifest). Short, free of typed content, and redacted by the
/// audit log like every field.
pub fn audit_detail(command: &Command) -> Option<String> {
    if let Some(detail) = crate::apply::audit_detail(command) {
        return Some(detail);
    }
    if let Some(detail) = super::selection_policy::detail(command) {
        return Some(detail);
    }
    if let Some(detail) = crate::helm_install::audit_detail(command) {
        return Some(detail);
    }
    if let Some(detail) = crate::helm_upgrade::audit_detail(command) {
        return Some(detail);
    }
    if let Some(detail) = crate::helm_ops::audit_detail(command) {
        return Some(detail);
    }
    if let Some(detail) = crate::role_ops::audit_detail(command) {
        return Some(detail);
    }
    if let Some(detail) = crate::sa_kubeconfig::audit_detail(command) {
        return Some(detail);
    }
    if let Some(detail) = crate::binding_ops::audit_detail(command) {
        return Some(detail);
    }
    match command {
        Command::WorkloadScale { replicas, .. } => {
            // The handler adds `before=` and `after=` (`Mutation::note`).
            return Some(format!("replicas={replicas}"));
        }
        // Label keys are not secrets, but the count is all a record needs.
        Command::NamespaceCreate { labels, .. } => {
            return Some(format!("labels={}", labels.len()));
        }
        // What the namespace held when the user asked, as `contents=Pod:12,Deployment:3`.
        Command::NamespaceDelete { contents, .. } if !contents.is_empty() => {
            // Kind names only (the caller supplies the keys): anything else is dropped.
            let held: Vec<String> = contents
                .iter()
                .filter(|(kind, _)| {
                    kind.len() <= 40 && kind.chars().all(|c| c.is_ascii_alphanumeric())
                })
                .take(32)
                .map(|(kind, n)| format!("{kind}:{n}"))
                .collect();
            if !held.is_empty() {
                return Some(format!("contents={}", held.join(",")));
            }
        }
        Command::PvcResize { size, from, .. } => {
            return Some(match from {
                Some(from) => format!("from={from} to={size}"),
                None => format!("to={size}"),
            });
        }
        Command::StorageClassSetDefault { clear_previous, .. } => {
            return Some(format!("clear_previous={clear_previous}"));
        }
        // The flags (names and numbers); the handler adds the batch id the per-pod records share
        // and, at the end, the counts.
        Command::NodeDrain { flags, .. } => return Some(flags.audit_words()),
        _ => {}
    }
    if let Command::WorkloadSetImage { images, .. } = command {
        // Which containers were asked for, even when the request fails before the handler can say
        // more; the handler adds each old and new image and the object's version.
        let names: Vec<_> = images
            .iter()
            .map(|change| change.container.as_str())
            .collect();
        return Some(format!("containers={}", names.join(",")));
    }
    if let Command::WorkloadRollback { to_revision, .. } = command {
        // The handler adds `from=` and `to=`.
        return Some(match to_revision {
            Some(revision) => format!("to_revision={revision}"),
            None => "to_revision=previous".to_owned(),
        });
    }
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

/// [`summary`], with what only the cluster's settings know: a node shell names the image and the
/// namespace of the privileged pod it will create, so the user confirms what actually runs as
/// root on the node.
pub fn summary_in(
    meta: &CommandMeta,
    command: &Command,
    context: &str,
    prefs: &ClusterPrefs,
) -> String {
    let base = summary(meta, command, context);
    match command {
        Command::NodeShell { target } => {
            let spec = NodeShellSpec::for_node(&*target.name, prefs);
            format!(
                "{base}. Creates a privileged pod from image {} in namespace {} with full access \
                 to the node; it is deleted when the terminal closes.",
                spec.image, spec.namespace
            )
        }
        _ => base,
    }
}

/// [`audit_detail`], with what only the cluster's settings know: the image and namespace of a node
/// shell's pod (`phase=create`; the record of its deletion says `phase=delete`). `None` for
/// every other command, which its id and target describe fully.
pub fn audit_detail_in(command: &Command, prefs: &ClusterPrefs) -> Option<String> {
    match command {
        Command::NodeShell { target } => {
            let spec = NodeShellSpec::for_node(&*target.name, prefs);
            Some(node_shell_detail("create", &spec))
        }
        _ => audit_detail(command),
    }
}

/// `phase=create image=busybox:1.37 namespace=kube-system`: the audit detail of one end of a
/// node shell's pod.
pub fn node_shell_detail(phase: &str, spec: &NodeShellSpec) -> String {
    format!(
        "phase={phase} image={} namespace={}",
        spec.image, spec.namespace
    )
}

/// `phase=sweep namespace=kube-system`: the audit detail of a leftover node-shell pod the sweep
/// deleted because nothing had stamped it alive for the grace period (its owner is gone).
pub fn node_shell_sweep_detail(namespace: &str) -> String {
    format!("phase=sweep namespace={namespace}")
}

/// The text a [`ConfirmTier::TypeName`] confirmation must repeat: the target's name, or
/// the cluster's context name for a command without one.
pub fn expected_name(command: &Command, context: &str) -> String {
    if let Some(release) = crate::helm_ops::expected_name(command) {
        return release;
    }
    command
        .target()
        .map_or_else(|| context.to_owned(), |t| t.name.to_string())
}
