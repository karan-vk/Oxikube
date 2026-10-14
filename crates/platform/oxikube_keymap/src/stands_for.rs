//! View actions that stand for commands (non-negotiable 4).
//!
//! An action whose name is a declared `CommandId` *is* that command ([`ActionRegistry::command`](crate::ActionRegistry::command)).
//! Most views declare their own actions instead (`resource_table::ViewYaml`, `log_view::ToggleWrap`)
//! because they carry the view's state: the row under the cursor, the log view's target. Such an
//! action's handler dispatches the command with that state, so the key, the toolbar, the palette
//! and an agent run one behaviour. This table is the other half of that contract: it names the
//! command (or commands, when the view picks by kind) each such action stands for, so
//!
//! - the palette (E11-S03) and the help overlay (E11-S10) can show the key next to the command
//!   ([`bindings_for_command`](crate::bindings_for_command)),
//! - a test can fail on a default binding that dispatches no command and is not view navigation.
//!
//! Actions that only move a cursor, a selection or the focus (`resource_table::SelectNext`,
//! `catalog::SelectFirst`) are not commands and are not listed.

use oxikube_domain::command::CommandId;

/// `(action, command)` pairs. An action may appear more than once when the view picks the command
/// by kind (`s` is `pod::Shell` on a Pod and `node::Shell` on a Node).
pub const STANDS_FOR: &[(&str, CommandId)] = &[
    // The resource table (E07-S03/S04/S08, E09-S08/S10, E11-S07).
    ("resource_table::OpenSelected", CommandId::RESOURCE_OPEN),
    ("resource_table::CopyName", CommandId::RESOURCE_COPY_NAME),
    ("resource_table::SelectAll", CommandId::RESOURCE_SELECT_ALL),
    ("resource_table::DeleteSelected", CommandId::RESOURCE_DELETE),
    // A Namespace is deleted by its own command (the type-the-name dialog with its warnings).
    (
        "resource_table::DeleteSelected",
        CommandId::NAMESPACE_DELETE,
    ),
    ("resource_table::DeleteSelected", CommandId::POD_DELETE),
    ("resource_table::FocusFilter", CommandId::TABLE_FOCUS_FILTER),
    ("resource_table::ShellSelected", CommandId::POD_SHELL),
    ("resource_table::ShellSelected", CommandId::NODE_SHELL),
    ("resource_table::AttachSelected", CommandId::POD_ATTACH),
    // `s` on a Deployment, StatefulSet, ReplicaSet or ReplicationController (E12-S04).
    ("resource_table::ScaleSelected", CommandId::WORKLOAD_SCALE),
    // `c` / `u` on the Nodes table (E12-S09).
    ("resource_table::CordonSelected", CommandId::NODE_CORDON),
    ("resource_table::UncordonSelected", CommandId::NODE_UNCORDON),
    // `r` and `i` on a Deployment, StatefulSet or DaemonSet (E12-S05).
    (
        "resource_table::RestartSelected",
        CommandId::WORKLOAD_RESTART,
    ),
    (
        "resource_table::SetImageSelected",
        CommandId::WORKLOAD_SET_IMAGE,
    ),
    // The same two keys in the focused detail drawer.
    ("resource_detail::Restart", CommandId::WORKLOAD_RESTART),
    ("resource_detail::SetImage", CommandId::WORKLOAD_SET_IMAGE),
    // `ctrl-l` on a Deployment, StatefulSet, DaemonSet or ReplicaSet (E12-S06).
    (
        "resource_table::RollbackSelected",
        CommandId::WORKLOAD_ROLLBACK,
    ),
    // `r` on the Nodes table (E12-S10): the drain dialog.
    ("resource_table::DrainSelected", CommandId::NODE_DRAIN),
    // `shift-l` on any table (E12-S19): the label editor over the selection.
    ("resource_table::EditLabels", CommandId::SELECTION_LABEL),
    ("resource_table::DebugSelected", CommandId::POD_DEBUG),
    ("resource_table::ViewYaml", CommandId::RESOURCE_VIEW_YAML),
    (
        "resource_table::ViewDescribe",
        CommandId::RESOURCE_VIEW_DESCRIBE,
    ),
    ("resource_table::EditSelected", CommandId::RESOURCE_EDIT),
    ("resource_table::ViewLogs", CommandId::POD_VIEW_LOGS),
    ("resource_table::ViewLogs", CommandId::WORKLOAD_VIEW_LOGS),
    ("resource_table::PortForward", CommandId::POD_PORT_FORWARD),
    // The Namespaces table (E12-S16): `u` uses the namespace under the cursor, `p` pins it; the
    // digits select All or the nth favourite in every table.
    ("resource_table::UseNamespace", CommandId::NAMESPACE_SELECT),
    (
        "resource_table::ToggleFavouriteNamespace",
        CommandId::NAMESPACE_TOGGLE_FAVOURITE,
    ),
    (
        "resource_table::SelectNamespaceSlot",
        CommandId::NAMESPACE_USE_FAVOURITE,
    ),
    // `resource_table::ShowPortForwards` (`f`) is deliberately absent: its handler only toasts
    // "not available yet" until port forwarding lands, so it stands for no command. Once it
    // dispatches `view::Open` with the view `port_forwards`, list it here and drop it from
    // `PLACEHOLDER_VERBS` in `tests/k9s.rs`.
    ("resource_table::ToggleWide", CommandId::TABLE_TOGGLE_WIDE),
    // CronJobs and Jobs (E12-S08): `t` triggers, `s` suspends or, when suspended, resumes.
    (
        "resource_table::TriggerSelected",
        CommandId::CRONJOB_TRIGGER,
    ),
    (
        "resource_table::ToggleSuspendSelected",
        CommandId::CRONJOB_SUSPEND,
    ),
    (
        "resource_table::ToggleSuspendSelected",
        CommandId::CRONJOB_RESUME,
    ),
    (
        "resource_table::ToggleSuspendSelected",
        CommandId::JOB_SUSPEND,
    ),
    (
        "resource_table::ToggleSuspendSelected",
        CommandId::JOB_RESUME,
    ),
    // A Service's verbs (E12-S13), bound where `kind == Service`.
    (
        "resource_table::ShowServicePods",
        CommandId::SERVICE_SHOW_PODS,
    ),
    (
        "resource_table::OpenInBrowser",
        CommandId::SERVICE_OPEN_IN_BROWSER,
    ),
    // Storage (E12-S15): `shift-r` resizes the claim under the cursor, `shift-s` makes the storage
    // class the default; each only on its own kind.
    ("resource_table::ResizeClaim", CommandId::PVC_RESIZE),
    (
        "resource_table::SetDefaultClass",
        CommandId::STORAGECLASS_SET_DEFAULT,
    ),
    // Used-by on the five referenced kinds (E12-S18).
    (
        "resource_table::ShowUsedBy",
        CommandId::RESOURCE_SHOW_USED_BY,
    ),
    // A Secret's reveal and decoded edit (E12-S11).
    ("resource_table::RevealSelected", CommandId::SECRET_REVEAL),
    (
        "resource_table::EditDecodedSelected",
        CommandId::SECRET_EDIT_DECODED,
    ),
    ("resource_detail::RevealSecret", CommandId::SECRET_REVEAL),
    (
        "resource_detail::EditSecretDecoded",
        CommandId::SECRET_EDIT_DECODED,
    ),
    // Pods (E12-S03): `ctrl-k` kills (force delete), `z` sanitizes, `o` shows the node, `shift-j`
    // jumps to the owner.
    ("resource_table::KillSelected", CommandId::POD_FORCE_DELETE),
    ("resource_table::SanitizeSelected", CommandId::POD_SANITIZE),
    ("resource_table::ShowNodeSelected", CommandId::POD_SHOW_NODE),
    (
        "resource_table::JumpToOwnerSelected",
        CommandId::POD_JUMP_TO_OWNER,
    ),
    // The resource detail's find in the YAML and Describe text (E11-S06). `resource_detail::CloseFind`
    // only closes the field and gives the keys back to the detail, so it is view-local.
    ("resource_detail::Find", CommandId::RESOURCE_FIND),
    ("resource_detail::NextMatch", CommandId::RESOURCE_NEXT_MATCH),
    (
        "resource_detail::PreviousMatch",
        CommandId::RESOURCE_PREVIOUS_MATCH,
    ),
    // The events view (E14-S03): `enter` opens the event's object, the rest are its commands.
    ("events_view::OpenSelected", CommandId::RESOURCE_OPEN),
    ("events_view::ToggleFaults", CommandId::EVENTS_TOGGLE_FAULTS),
    ("events_view::TogglePause", CommandId::EVENTS_TOGGLE_PAUSE),
    ("events_view::CopyRow", CommandId::EVENTS_COPY_ROW),
    ("events_view::CopyRowJson", CommandId::EVENTS_COPY_ROW_JSON),
    ("events_view::ExportJson", CommandId::EVENTS_EXPORT_JSON),
    // The pulses page (E13-S10): `enter` opens the selected tile's table.
    ("pulses::Open", CommandId::RESOURCE_OPEN_LIST),
    // The notification centre (E14-S05): `enter` marks the selected notification read and opens
    // its object, `r` marks it read.
    (
        "notification_panel::OpenSelected",
        CommandId::NOTIFICATIONS_MARK_READ,
    ),
    ("notification_panel::OpenSelected", CommandId::RESOURCE_OPEN),
    (
        "notification_panel::MarkSelectedRead",
        CommandId::NOTIFICATIONS_MARK_READ,
    ),
    // The catalog home (E06-S03).
    ("catalog::ConnectSelected", CommandId::CLUSTER_CONNECT),
    ("catalog::DisconnectSelected", CommandId::CLUSTER_DISCONNECT),
    (
        "catalog::ToggleFavouriteSelected",
        CommandId::CLUSTER_TOGGLE_FAVOURITE,
    ),
    // The log viewer (E08).
    ("log_view::Tail", CommandId::LOGS_SET_RANGE),
    ("log_view::Head", CommandId::LOGS_SET_RANGE),
    ("log_view::Since1m", CommandId::LOGS_SET_RANGE),
    ("log_view::Since5m", CommandId::LOGS_SET_RANGE),
    ("log_view::Since15m", CommandId::LOGS_SET_RANGE),
    ("log_view::Since30m", CommandId::LOGS_SET_RANGE),
    ("log_view::Since1h", CommandId::LOGS_SET_RANGE),
    (
        "log_view::ToggleAutoscroll",
        CommandId::LOGS_TOGGLE_AUTOSCROLL,
    ),
    ("log_view::ToggleWrap", CommandId::LOGS_TOGGLE_WRAP),
    (
        "log_view::ToggleTimestamps",
        CommandId::LOGS_TOGGLE_TIMESTAMPS,
    ),
    ("log_view::ToggleJsonMode", CommandId::LOGS_TOGGLE_JSON_MODE),
    ("log_view::TogglePrevious", CommandId::LOGS_TOGGLE_PREVIOUS),
    (
        "log_view::ToggleFullscreen",
        CommandId::LOGS_TOGGLE_FULLSCREEN,
    ),
    ("log_view::Mark", CommandId::LOGS_MARK),
    ("log_view::Copy", CommandId::LOGS_COPY),
    ("log_view::Clear", CommandId::LOGS_CLEAR),
    ("log_view::SendToAgent", CommandId::LOGS_SEND_TO_AGENT),
    ("log_view::TailInTerminal", CommandId::LOGS_TAIL_IN_TERMINAL),
    ("log_view::SaveAll", CommandId::LOGS_SAVE),
    ("log_view::SaveVisible", CommandId::LOGS_SAVE),
    ("log_view::Reconnect", CommandId::LOGS_RECONNECT),
    (
        "log_view::FollowReplacement",
        CommandId::LOGS_FOLLOW_REPLACEMENT,
    ),
    ("log_view::Find", CommandId::LOGS_FIND),
    ("log_view::NextMatch", CommandId::LOGS_NEXT_MATCH),
    ("log_view::PreviousMatch", CommandId::LOGS_PREVIOUS_MATCH),
    ("log_view::CloseSearch", CommandId::LOGS_CLOSE_SEARCH),
    ("log_view::ToggleCase", CommandId::LOGS_TOGGLE_CASE),
    ("log_view::ToggleInverse", CommandId::LOGS_TOGGLE_INVERSE),
    (
        "log_view::ToggleFilterMode",
        CommandId::LOGS_TOGGLE_FILTER_MODE,
    ),
];

/// The commands the view action `name` stands for (empty when it is none).
pub fn commands_of_action(name: &str) -> impl Iterator<Item = CommandId> + '_ {
    STANDS_FOR
        .iter()
        .filter(move |(action, _)| *action == name)
        .map(|(_, command)| *command)
}

/// The action names that stand for `command`, not counting an action named like the command.
pub fn view_actions_of(command: CommandId) -> impl Iterator<Item = &'static str> {
    STANDS_FOR
        .iter()
        .filter(move |(_, id)| *id == command)
        .map(|(action, _)| *action)
}

#[cfg(test)]
mod tests {
    use oxikube_domain::command;

    use super::*;

    #[test]
    fn every_listed_command_is_declared_and_no_pair_repeats() {
        for (i, (action, id)) in STANDS_FOR.iter().enumerate() {
            assert!(
                command::lookup(*id).is_some(),
                "{action} stands for an undeclared command {id}"
            );
            assert!(
                !STANDS_FOR[..i].contains(&(*action, *id)),
                "{action} -> {id} is listed twice"
            );
            assert!(
                command::lookup_str(action).is_none(),
                "{action} is itself a command id: it needs no entry"
            );
        }
    }

    #[test]
    fn an_action_can_stand_for_one_command_per_kind() {
        let shell: Vec<_> = commands_of_action("resource_table::ShellSelected").collect();
        assert_eq!(shell, [CommandId::POD_SHELL, CommandId::NODE_SHELL]);
        assert_eq!(
            view_actions_of(CommandId::TABLE_TOGGLE_WIDE).collect::<Vec<_>>(),
            ["resource_table::ToggleWide"]
        );
        assert_eq!(commands_of_action("resource_table::SelectNext").count(), 0);
    }
}
