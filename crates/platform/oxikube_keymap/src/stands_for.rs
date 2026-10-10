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
    ("resource_table::FocusFilter", CommandId::TABLE_FOCUS_FILTER),
    ("resource_table::ShellSelected", CommandId::POD_SHELL),
    ("resource_table::ShellSelected", CommandId::NODE_SHELL),
    ("resource_table::AttachSelected", CommandId::POD_ATTACH),
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
    // `resource_table::ShowPortForwards` (`f`) is deliberately absent: its handler only toasts
    // "not available yet" until port forwarding lands, so it stands for no command. Once it
    // dispatches `view::Open` with the view `port_forwards`, list it here and drop it from
    // `PLACEHOLDER_VERBS` in `tests/k9s.rs`.
    ("resource_table::ToggleWide", CommandId::TABLE_TOGGLE_WIDE),
    // The resource detail's find in the YAML and Describe text (E11-S06). `resource_detail::CloseFind`
    // only closes the field and gives the keys back to the detail, so it is view-local.
    ("resource_detail::Find", CommandId::RESOURCE_FIND),
    ("resource_detail::NextMatch", CommandId::RESOURCE_NEXT_MATCH),
    (
        "resource_detail::PreviousMatch",
        CommandId::RESOURCE_PREVIOUS_MATCH,
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
