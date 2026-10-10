//! `commands_for`: from a command id and a target to the payloads the bus dispatches.

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};

use crate::command_bus::{CommandTarget, InvokeError, commands_for};

fn cluster() -> ClusterId {
    ClusterId::new("/kube/config", &ContextName::new("kind-test"))
}

fn pod(name: &str) -> ResourceRef {
    ResourceRef::new(
        cluster(),
        Gvk::new("", "v1", "Pod"),
        Some("default".into()),
        name,
    )
}

#[test]
fn a_command_without_operands_needs_no_target() {
    let commands = commands_for(CommandId::VIEW_ZOOM_IN, &CommandTarget::none()).unwrap();
    assert_eq!(commands, [Command::ViewZoomIn]);
    let commands = commands_for(CommandId::PALETTE_TOGGLE, &CommandTarget::none()).unwrap();
    assert_eq!(commands, [Command::PaletteToggle]);
}

#[test]
fn an_unrelated_target_does_not_change_a_command_without_operands() {
    let target = CommandTarget::none()
        .in_cluster(cluster())
        .selecting(vec![pod("a"), pod("b")]);
    let commands = commands_for(CommandId::VIEW_ZOOM_IN, &target).unwrap();
    assert_eq!(commands, [Command::ViewZoomIn], "once, not once per object");
}

#[test]
fn a_command_on_objects_runs_once_per_selected_object() {
    let target = CommandTarget::none()
        .in_cluster(cluster())
        .selecting(vec![pod("a"), pod("b")]);
    let commands = commands_for(CommandId::POD_VIEW_LOGS, &target).unwrap();
    let names: Vec<_> = commands
        .iter()
        .map(|command| match command {
            Command::PodViewLogs { target, .. } => target.name.to_string(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(names, ["a", "b"]);
}

#[test]
fn a_command_on_the_kind_takes_the_cluster_and_kind_of_the_view() {
    let gvk = Gvk::new("apps", "v1", "Deployment");
    let target = CommandTarget::none()
        .in_cluster(cluster())
        .of_kind(gvk.clone());
    let commands = commands_for(CommandId::RESOURCE_OPEN_LIST, &target).unwrap();
    assert_eq!(
        commands,
        [Command::ResourceOpenList {
            cluster: cluster(),
            gvk
        }]
    );
}

#[test]
fn a_missing_operand_is_a_request_for_input_not_a_guess() {
    // No object selected for a command on objects.
    let error = commands_for(CommandId::POD_DELETE, &CommandTarget::none()).unwrap_err();
    assert!(matches!(error, InvokeError::NeedsInput { id, .. } if id == CommandId::POD_DELETE));
    // An operand only the user can give (the replica count).
    let target = CommandTarget::none()
        .in_cluster(cluster())
        .selecting(vec![pod("a")]);
    let error = commands_for(CommandId::WORKLOAD_SCALE, &target).unwrap_err();
    let InvokeError::NeedsInput { detail, .. } = error else {
        panic!("{error:?}");
    };
    assert!(detail.contains("replicas"), "{detail}");
}

#[test]
fn an_undeclared_id_is_reported() {
    let id = CommandId::new("nope::Nothing");
    assert_eq!(
        commands_for(id, &CommandTarget::none()).unwrap_err(),
        InvokeError::Unknown(id)
    );
}

/// Every declared command either builds from a full target or says what it lacks: none panics,
/// and the command built has the id asked for.
#[test]
fn every_declared_command_builds_or_asks_for_input() {
    let target = CommandTarget::none()
        .in_cluster(cluster())
        .of_kind(Gvk::new("", "v1", "Pod"))
        .selecting(vec![pod("a")]);
    let mut built = 0;
    for meta in command::COMMANDS {
        match commands_for(meta.id, &target) {
            Ok(commands) => {
                built += 1;
                assert!(!commands.is_empty(), "{}", meta.id);
                for command in commands {
                    assert_eq!(command.id(), meta.id);
                }
            }
            Err(InvokeError::NeedsInput { id, .. }) => assert_eq!(id, meta.id),
            Err(other) => panic!("{}: {other}", meta.id),
        }
    }
    assert!(built > 30, "most commands build from a selection: {built}");
}

/// A select-all of a 10k-pod table is turned into commands in time linear in the selection: the
/// palette does it on the UI thread (the dedup of identical commands used to be quadratic).
#[test]
fn a_select_all_of_ten_thousand_objects_builds_in_linear_time() {
    let pods: Vec<_> = (0..10_000).map(|i| pod(&format!("pod-{i}"))).collect();
    let target = CommandTarget::none().in_cluster(cluster()).selecting(pods);
    let started = std::time::Instant::now();
    let per_object = commands_for(CommandId::POD_VIEW_LOGS, &target).unwrap();
    let once = commands_for(CommandId::VIEW_ZOOM_IN, &target).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(per_object.len(), 10_000, "one per selected object");
    assert_eq!(once, [Command::ViewZoomIn], "once, not once per object");
    // Quadratic dedup takes tens of seconds here in a debug build; linear takes well under one.
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "10k objects took {elapsed:?}"
    );
}
