//! The static registry of declared commands: [`COMMANDS`], [`lookup`] and the
//! [`CommandId`] constants.
//!
//! One entry per [`Command`](super::Command) variant. Adding a command means a
//! new constant here, a registry row (keep the slice sorted by id), a variant
//! in `payload.rs`, and a handler in `oxikube_app`. The tests in this module
//! catch an unsorted slice, a duplicate id, a malformed id, a colliding tool
//! name and a mutating command without a deliberate confirmation tier.

use super::capability::Capabilities;
use super::id::CommandId;
use super::meta::{CommandMeta, CommandScope};
use crate::safety::Risk;

impl CommandId {
    /// `app::Quit`: quit the application (confirms first while operations run).
    pub const APP_QUIT: CommandId = CommandId::new("app::Quit");
    /// `cluster::ApplyPreset`: give a cluster a prod / staging / dev / none posture.
    pub const CLUSTER_APPLY_PRESET: CommandId = CommandId::new("cluster::ApplyPreset");
    /// `cluster::CancelConnect`: cancel a connection attempt that is still in flight.
    pub const CLUSTER_CANCEL_CONNECT: CommandId = CommandId::new("cluster::CancelConnect");
    /// `cluster::CloseTab`: close a cluster's tab (disconnects it).
    pub const CLUSTER_CLOSE_TAB: CommandId = CommandId::new("cluster::CloseTab");
    /// `cluster::Connect`: connect a cluster (open its session).
    pub const CLUSTER_CONNECT: CommandId = CommandId::new("cluster::Connect");
    /// `cluster::Disconnect`: disconnect a cluster or cancel the attempt.
    pub const CLUSTER_DISCONNECT: CommandId = CommandId::new("cluster::Disconnect");
    /// `cluster::NextTab`: show the next cluster tab.
    pub const CLUSTER_NEXT_TAB: CommandId = CommandId::new("cluster::NextTab");
    /// `cluster::PreviousTab`: show the previous cluster tab.
    pub const CLUSTER_PREVIOUS_TAB: CommandId = CommandId::new("cluster::PreviousTab");
    /// `cluster::Reconnect`: drop a cluster's connection and connect it again (the retry).
    pub const CLUSTER_RECONNECT: CommandId = CommandId::new("cluster::Reconnect");
    /// `cluster::Select`: make a cluster the active one.
    pub const CLUSTER_SELECT: CommandId = CommandId::new("cluster::Select");
    /// `cluster::SetColour`: set or clear a cluster's accent colour.
    pub const CLUSTER_SET_COLOUR: CommandId = CommandId::new("cluster::SetColour");
    /// `cluster::SwitchTab`: show the nth cluster tab.
    pub const CLUSTER_SWITCH_TAB: CommandId = CommandId::new("cluster::SwitchTab");
    /// `cluster::ToggleFavourite`: mark or unmark a cluster as a favourite.
    pub const CLUSTER_TOGGLE_FAVOURITE: CommandId = CommandId::new("cluster::ToggleFavourite");
    /// `cluster::ToggleReadOnly`: set or toggle a cluster's read-only mode.
    pub const CLUSTER_TOGGLE_READ_ONLY: CommandId = CommandId::new("cluster::ToggleReadOnly");
    /// `kubeconfig::AddSource`: add a kubeconfig file, directory or pasted text as a source.
    pub const KUBECONFIG_ADD_SOURCE: CommandId = CommandId::new("kubeconfig::AddSource");
    /// `kubeconfig::Reload`: re-read every kubeconfig source.
    pub const KUBECONFIG_RELOAD: CommandId = CommandId::new("kubeconfig::Reload");
    /// `kubeconfig::RemoveSource`: remove a kubeconfig source.
    pub const KUBECONFIG_REMOVE_SOURCE: CommandId = CommandId::new("kubeconfig::RemoveSource");
    /// `namespace::Select`: choose the namespace selection.
    pub const NAMESPACE_SELECT: CommandId = CommandId::new("namespace::Select");
    /// `namespace::ToggleFavourite`: pin or unpin a namespace as a favourite.
    pub const NAMESPACE_TOGGLE_FAVOURITE: CommandId = CommandId::new("namespace::ToggleFavourite");
    /// `node::Cordon`: mark a node unschedulable.
    pub const NODE_CORDON: CommandId = CommandId::new("node::Cordon");
    /// `node::Drain`: evict a node's pods.
    pub const NODE_DRAIN: CommandId = CommandId::new("node::Drain");
    /// `node::Uncordon`: mark a node schedulable again.
    pub const NODE_UNCORDON: CommandId = CommandId::new("node::Uncordon");
    /// `palette::Toggle`: show or hide the command palette.
    pub const PALETTE_TOGGLE: CommandId = CommandId::new("palette::Toggle");
    /// `pod::Delete`: delete one pod.
    pub const POD_DELETE: CommandId = CommandId::new("pod::Delete");
    /// `pod::Exec`: run a command (or shell) in a container.
    pub const POD_EXEC: CommandId = CommandId::new("pod::Exec");
    /// `pod::PortForward`: forward a local port to a pod port.
    pub const POD_PORT_FORWARD: CommandId = CommandId::new("pod::PortForward");
    /// `pod::ViewLogs`: open a pod's logs.
    pub const POD_VIEW_LOGS: CommandId = CommandId::new("pod::ViewLogs");
    /// `resource::Apply`: apply a manifest.
    pub const RESOURCE_APPLY: CommandId = CommandId::new("resource::Apply");
    /// `resource::CopyLabel`: copy a label or annotation of a resource as `key=value`.
    pub const RESOURCE_COPY_LABEL: CommandId = CommandId::new("resource::CopyLabel");
    /// `resource::CopyName`: copy a resource's name to the clipboard.
    pub const RESOURCE_COPY_NAME: CommandId = CommandId::new("resource::CopyName");
    /// `resource::Delete`: delete any resource.
    pub const RESOURCE_DELETE: CommandId = CommandId::new("resource::Delete");
    /// `resource::Open`: open a resource's detail view.
    pub const RESOURCE_OPEN: CommandId = CommandId::new("resource::Open");
    /// `resource::OpenList`: open the list view of a resource kind.
    pub const RESOURCE_OPEN_LIST: CommandId = CommandId::new("resource::OpenList");
    /// `resource::RetryFeed`: restart the feed behind a kind's list views.
    pub const RESOURCE_RETRY_FEED: CommandId = CommandId::new("resource::RetryFeed");
    /// `resource::PinDetail`: promote a resource's detail drawer to a workspace tab.
    pub const RESOURCE_PIN_DETAIL: CommandId = CommandId::new("resource::PinDetail");
    /// `resource::SelectAll`: select every row of a kind's list views.
    pub const RESOURCE_SELECT_ALL: CommandId = CommandId::new("resource::SelectAll");
    /// `resource::ViewYaml`: open a resource's YAML.
    pub const RESOURCE_VIEW_YAML: CommandId = CommandId::new("resource::ViewYaml");
    /// `table::FocusFilter`: move the keyboard focus to a resource table's filter bar.
    pub const TABLE_FOCUS_FILTER: CommandId = CommandId::new("table::FocusFilter");
    /// `view::Open`: open a registered view by id.
    pub const VIEW_OPEN: CommandId = CommandId::new("view::Open");
    /// `view::ZoomIn`: make the UI one zoom step larger.
    pub const VIEW_ZOOM_IN: CommandId = CommandId::new("view::ZoomIn");
    /// `view::ZoomOut`: make the UI one zoom step smaller.
    pub const VIEW_ZOOM_OUT: CommandId = CommandId::new("view::ZoomOut");
    /// `view::ZoomReset`: set the UI zoom back to 100 %.
    pub const VIEW_ZOOM_RESET: CommandId = CommandId::new("view::ZoomReset");
    /// `window::New`: open another main window.
    pub const WINDOW_NEW: CommandId = CommandId::new("window::New");
    /// `workload::Restart`: rolling-restart a workload.
    pub const WORKLOAD_RESTART: CommandId = CommandId::new("workload::Restart");
    /// `workload::Scale`: set a workload's replica count.
    pub const WORKLOAD_SCALE: CommandId = CommandId::new("workload::Scale");
}

const NONE: Capabilities = Capabilities::empty();

/// Every declared command, **sorted by id** (lookup is a binary search; a test
/// enforces the order).
pub static COMMANDS: &[CommandMeta] = &[
    CommandMeta::read(
        CommandId::APP_QUIT,
        "Quit Oxikube",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_APPLY_PRESET,
        "Apply Cluster Preset",
        CommandScope::Cluster,
        NONE,
    ),
    // Gives up an attempt in flight; nothing is read or changed in the cluster.
    CommandMeta::read(
        CommandId::CLUSTER_CANCEL_CONNECT,
        "Cancel Connecting",
        CommandScope::Global,
        NONE,
    ),
    // Closing a tab disconnects: it reads nothing and changes nothing in the cluster, so it is no
    // mutation. The confirmation it may show (running operations) is a UI prompt, not a guard tier.
    CommandMeta::read(
        CommandId::CLUSTER_CLOSE_TAB,
        "Close Cluster Tab",
        CommandScope::Global,
        NONE,
    ),
    // Connecting reads from the cluster and changes nothing in it: not `mutating`, no guard tier.
    CommandMeta::read(
        CommandId::CLUSTER_CONNECT,
        "Connect Cluster",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_DISCONNECT,
        "Disconnect Cluster",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_NEXT_TAB,
        "Next Cluster Tab",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_PREVIOUS_TAB,
        "Previous Cluster Tab",
        CommandScope::Global,
        NONE,
    ),
    // Retrying a connection reads from the cluster like `cluster::Connect`: no guard tier.
    CommandMeta::read(
        CommandId::CLUSTER_RECONNECT,
        "Reconnect Cluster",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_SELECT,
        "Select Cluster",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_SET_COLOUR,
        "Set Cluster Colour",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_SWITCH_TAB,
        "Switch Cluster Tab",
        CommandScope::Global,
        NONE,
    ),
    // Local catalog preference (StatePort), never a cluster change.
    CommandMeta::read(
        CommandId::CLUSTER_TOGGLE_FAVOURITE,
        "Toggle Favourite Cluster",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::privileged(
        CommandId::CLUSTER_TOGGLE_READ_ONLY,
        "Toggle Read-Only Mode",
        CommandScope::Cluster,
        NONE,
    ),
    // The kubeconfig commands change the user's settings list and Oxikube's own files, never a
    // cluster: not `mutating`, no guard tier. Removing a pasted kubeconfig deletes a file, so
    // the UI confirms it first and the handler refuses it for agents.
    CommandMeta::read(
        CommandId::KUBECONFIG_ADD_SOURCE,
        "Add Kubeconfig Source",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::KUBECONFIG_RELOAD,
        "Reload Kubeconfigs",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::KUBECONFIG_REMOVE_SOURCE,
        "Remove Kubeconfig Source",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::NAMESPACE_SELECT,
        "Select Namespaces",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::read(
        CommandId::NAMESPACE_TOGGLE_FAVOURITE,
        "Toggle Favourite Namespace",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::mutation(
        CommandId::NODE_CORDON,
        "Cordon Node",
        CommandScope::Selection,
        Risk::Medium,
        NONE,
    ),
    CommandMeta::mutation(
        CommandId::NODE_DRAIN,
        "Drain Node",
        CommandScope::Selection,
        Risk::High,
        NONE,
    ),
    CommandMeta::mutation(
        CommandId::NODE_UNCORDON,
        "Uncordon Node",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    ),
    CommandMeta::read(
        CommandId::PALETTE_TOGGLE,
        "Toggle Command Palette",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::mutation(
        CommandId::POD_DELETE,
        "Delete Pod",
        CommandScope::Selection,
        Risk::Medium,
        NONE,
    ),
    CommandMeta::mutation(
        CommandId::POD_EXEC,
        "Exec into Container",
        CommandScope::Selection,
        Risk::Medium,
        Capabilities::EXEC,
    ),
    CommandMeta::read(
        CommandId::POD_PORT_FORWARD,
        "Port-Forward",
        CommandScope::Selection,
        Capabilities::PORTFORWARD,
    ),
    CommandMeta::read(
        CommandId::POD_VIEW_LOGS,
        "View Logs",
        CommandScope::Selection,
        Capabilities::LOGS,
    ),
    CommandMeta::mutation(
        CommandId::RESOURCE_APPLY,
        "Apply Manifest",
        CommandScope::Cluster,
        Risk::Medium,
        NONE,
    ),
    // Writes the user's clipboard, never the cluster.
    CommandMeta::read(
        CommandId::RESOURCE_COPY_LABEL,
        "Copy Label",
        CommandScope::Selection,
        NONE,
    ),
    // Writes the user's clipboard, never the cluster.
    CommandMeta::read(
        CommandId::RESOURCE_COPY_NAME,
        "Copy Resource Name",
        CommandScope::Selection,
        NONE,
    ),
    CommandMeta::mutation(
        CommandId::RESOURCE_DELETE,
        "Delete Resource",
        CommandScope::Selection,
        // The floor: the guard raises it by target (Namespace, Node, PersistentVolume, a cascading
        // delete: `Command::effective_risk`), so an ordinary object takes a simple confirm.
        Risk::Medium,
        NONE,
    ),
    CommandMeta::read(
        CommandId::RESOURCE_OPEN,
        "Open Resource",
        CommandScope::Selection,
        NONE,
    ),
    CommandMeta::read(
        CommandId::RESOURCE_OPEN_LIST,
        "Open Resource List",
        CommandScope::ResourceKind,
        NONE,
    ),
    // Moves a view between a drawer and a tab; changes nothing in the cluster.
    CommandMeta::read(
        CommandId::RESOURCE_PIN_DETAIL,
        "Pin Detail as Tab",
        CommandScope::Selection,
        NONE,
    ),
    CommandMeta::read(
        CommandId::RESOURCE_RETRY_FEED,
        "Retry Resource Feed",
        CommandScope::ResourceKind,
        NONE,
    ),
    CommandMeta::read(
        CommandId::RESOURCE_SELECT_ALL,
        "Select All Resources",
        CommandScope::ResourceKind,
        NONE,
    ),
    CommandMeta::read(
        CommandId::RESOURCE_VIEW_YAML,
        "View YAML",
        CommandScope::Selection,
        NONE,
    ),
    // Moves focus inside a window, never touches the cluster: allowed in read-only mode.
    CommandMeta::read(
        CommandId::TABLE_FOCUS_FILTER,
        "Focus Table Filter",
        CommandScope::ResourceKind,
        NONE,
    ),
    CommandMeta::read(
        CommandId::VIEW_OPEN,
        "Open View",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::VIEW_ZOOM_IN,
        "Zoom In",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::VIEW_ZOOM_OUT,
        "Zoom Out",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::VIEW_ZOOM_RESET,
        "Reset Zoom",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::WINDOW_NEW,
        "New Window",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::mutation(
        CommandId::WORKLOAD_RESTART,
        "Restart Workload",
        CommandScope::Selection,
        Risk::Medium,
        NONE,
    ),
    CommandMeta::mutation(
        CommandId::WORKLOAD_SCALE,
        "Scale Workload",
        CommandScope::Selection,
        Risk::Medium,
        NONE,
    ),
];

/// Find a command's metadata by id (binary search, no allocation).
pub fn lookup(id: CommandId) -> Option<&'static CommandMeta> {
    lookup_str(id.as_str())
}

/// Find a command's metadata by its `namespace::Verb` string.
pub fn lookup_str(id: &str) -> Option<&'static CommandMeta> {
    COMMANDS
        .binary_search_by(|meta| meta.id.as_str().cmp(id))
        .ok()
        .map(|i| &COMMANDS[i])
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::safety::{ConfirmTier, Initiator};

    /// Mutating commands that deliberately run without a confirmation. Empty on
    /// purpose: adding an entry here is a reviewed decision that a mutation is
    /// safe to run unconfirmed (say why in a comment next to the entry).
    const UNCONFIRMED_MUTATING: &[&str] = &[];

    #[test]
    fn ids_are_well_formed_unique_and_sorted() {
        let mut seen = HashSet::new();
        for pair in COMMANDS.windows(2) {
            assert!(
                pair[0].id < pair[1].id,
                "registry must be sorted by id: {} before {}",
                pair[0].id,
                pair[1].id
            );
        }
        for meta in COMMANDS {
            assert!(
                crate::command::is_well_formed(meta.id.as_str()),
                "{}",
                meta.id
            );
            assert!(seen.insert(meta.id), "duplicate id {}", meta.id);
            assert!(!meta.title.trim().is_empty(), "{} has no title", meta.id);
        }
    }

    #[test]
    fn tool_names_are_unique() {
        let mut seen = HashSet::new();
        for meta in COMMANDS {
            let tool = meta.id.tool_name();
            assert!(
                tool.starts_with("k8s.") || tool.starts_with("app."),
                "{tool}"
            );
            assert!(seen.insert(tool.clone()), "duplicate tool name {tool}");
        }
    }

    #[test]
    fn mutating_commands_are_guarded() {
        for meta in COMMANDS {
            if meta.mutating {
                assert!(
                    meta.needs.contains(Capabilities::MUTATE),
                    "{} mutates but does not need MUTATE",
                    meta.id
                );
                assert!(!meta.privileged, "{} is mutating and privileged", meta.id);
                let risk = meta.risk.expect("mutating command declares a risk");
                assert_eq!(meta.confirm, risk.confirm_tier(), "{}", meta.id);
                if meta.confirm == ConfirmTier::None {
                    assert!(
                        UNCONFIRMED_MUTATING.contains(&meta.id.as_str()),
                        "{} mutates with no confirmation and is not on the allow list",
                        meta.id
                    );
                }
            } else {
                assert_eq!(meta.confirm, ConfirmTier::None, "{}", meta.id);
                assert_eq!(meta.risk, None, "{}", meta.id);
                assert!(!meta.needs.contains(Capabilities::MUTATE), "{}", meta.id);
            }
        }
        for id in UNCONFIRMED_MUTATING {
            let meta = lookup_str(id).expect("allow-list entry is a registered id");
            assert!(meta.mutating, "{id} is on the allow list but not mutating");
        }
    }

    #[test]
    fn read_only_toggle_is_privileged_and_refused_for_agents() {
        let meta = lookup(CommandId::CLUSTER_TOGGLE_READ_ONLY).unwrap();
        assert!(meta.privileged);
        assert!(!meta.mutating, "must stay runnable on a read-only cluster");
        assert!(meta.allows(Initiator::Ui));
        assert!(meta.allows(Initiator::Command));
        assert!(!meta.allows(Initiator::Agent));
        assert!(!meta.allows(Initiator::Plugin));
        let privileged: Vec<_> = COMMANDS.iter().filter(|m| m.privileged).collect();
        assert_eq!(privileged.len(), 1, "privileged is a reviewed allow-list");
        let delete = lookup(CommandId::POD_DELETE).unwrap();
        assert!(delete.allows(Initiator::Agent));
    }

    #[test]
    fn connecting_is_a_read_command_with_a_tool_name() {
        // Connect and disconnect read from the cluster and change nothing in it (E06-S03): they
        // stay runnable on a read-only cluster and need no confirmation.
        for (id, tool) in [
            (CommandId::CLUSTER_CONNECT, "app.cluster_connect"),
            (CommandId::CLUSTER_DISCONNECT, "app.cluster_disconnect"),
            (CommandId::CLUSTER_RECONNECT, "app.cluster_reconnect"),
            (
                CommandId::CLUSTER_CANCEL_CONNECT,
                "app.cluster_cancel_connect",
            ),
            (
                CommandId::CLUSTER_TOGGLE_FAVOURITE,
                "app.cluster_toggle_favourite",
            ),
        ] {
            let meta = lookup(id).expect("registered");
            assert!(!meta.mutating && !meta.privileged, "{id}");
            assert_eq!(meta.confirm, ConfirmTier::None, "{id}");
            assert_eq!(id.tool_name(), tool);
        }
    }

    #[test]
    fn lookup_finds_every_entry() {
        for meta in COMMANDS {
            assert_eq!(lookup(meta.id), Some(meta));
        }
        assert_eq!(lookup_str("pod::Explode"), None);
        assert_eq!(lookup_str(""), None);
    }

    #[test]
    fn spot_check_declared_tiers() {
        let get = |id: CommandId| lookup(id).unwrap();
        assert_eq!(get(CommandId::RESOURCE_DELETE).confirm, ConfirmTier::Simple);
        assert_eq!(get(CommandId::NODE_DRAIN).confirm, ConfirmTier::TypeName);
        assert_eq!(get(CommandId::WORKLOAD_SCALE).confirm, ConfirmTier::Simple);
        assert_eq!(get(CommandId::POD_VIEW_LOGS).needs, Capabilities::LOGS);
        assert_eq!(
            get(CommandId::POD_EXEC).needs,
            Capabilities::EXEC | Capabilities::MUTATE
        );
        assert!(!get(CommandId::POD_PORT_FORWARD).mutating);
    }
}
