//! The exec class (E09-S08): `pod::Shell`, `pod::Attach` and `pod::Exec` are blocked on a
//! read-only cluster unless it allows them, are never confirmed, and are audited on every open
//! with the container and program, never the content.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{COMMANDS, Command, CommandId};
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::ClusterPrefs;

use crate::command_bus::{DispatchContext, DispatchError, Outcome};
use crate::testing::{Harness, cluster_context, ctx, id, pod};

const EXEC: [CommandId; 3] = [
    CommandId::POD_SHELL,
    CommandId::POD_ATTACH,
    CommandId::POD_EXEC,
];

fn shell(container: Option<&str>) -> Command {
    Command::PodShell {
        target: pod("a", "web-0"),
        container: container.map(str::to_owned),
    }
}

fn exec(argv: &[&str]) -> Command {
    Command::PodExec {
        target: pod("a", "web-0"),
        container: Some("app".into()),
        command: argv.iter().map(|a| (*a).to_owned()).collect(),
    }
}

/// A harness whose cluster `a` is opened from settings: read-only, with `exec_in_read_only`.
fn read_only_with(exec_in_read_only: bool) -> Harness {
    let h = Harness::with_every_command();
    h.prefs.seed(
        &id("a"),
        ClusterPrefs {
            read_only: true,
            exec_in_read_only,
            ..ClusterPrefs::default()
        },
    );
    h.connect_configured("a");
    assert!(h.manager.get(&id("a")).unwrap().read_only());
    h
}

#[test]
fn the_exec_commands_are_exactly_the_exec_class() {
    let class: Vec<_> = COMMANDS.iter().filter(|m| m.exec).map(|m| m.id).collect();
    let mut expected = EXEC.to_vec();
    expected.sort();
    assert_eq!(class, expected);
}

#[test]
fn a_read_only_cluster_blocks_a_shell_by_default_for_every_initiator() {
    let h = read_only_with(false);
    for id_ in EXEC {
        for initiator in Initiator::ALL {
            let command = crate::testing_posture::sample(id_, "a");
            let err = h.dispatch(command, ctx(initiator)).unwrap_err();
            assert!(
                matches!(err, DispatchError::ReadOnly { .. }),
                "{id_} as {initiator}: {err:?}"
            );
            assert_eq!(OxiError::from(err).kind(), ErrorKind::Forbidden);
        }
    }
    assert!(h.calls().is_empty(), "no handler ran: nothing was opened");
    let audit = h.audit();
    assert_eq!(audit.len(), EXEC.len() * Initiator::ALL.len());
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));
    assert_eq!(h.bus.guard().pending_confirmations(), 0, "it never asks");
}

#[test]
fn the_setting_lets_a_shell_open_on_a_read_only_cluster() {
    let h = read_only_with(true);
    for id_ in EXEC {
        let command = crate::testing_posture::sample(id_, "a");
        let out = h.dispatch(command, ctx(Initiator::Ui)).unwrap();
        assert!(matches!(out, Outcome::Completed(_)), "{id_}: {out:?}");
    }
    let calls = h.calls();
    assert_eq!(calls.len(), EXEC.len());
    assert!(
        calls.iter().all(|c| !c.mutation),
        "an exec handler never gets a write permit"
    );
    assert!(
        h.writes("a").is_empty(),
        "nothing was written to the cluster"
    );
    assert!(
        h.outcomes()
            .iter()
            .all(|(o, _)| *o == AuditOutcome::Succeeded)
    );
}

#[test]
fn the_setting_applies_at_once_in_both_directions() {
    let h = read_only_with(false);
    assert!(h.dispatch(shell(None), ctx(Initiator::Ui)).is_err());
    h.prefs.seed(
        &id("a"),
        ClusterPrefs {
            read_only: true,
            exec_in_read_only: true,
            ..ClusterPrefs::default()
        },
    );
    assert!(h.dispatch(shell(None), ctx(Initiator::Ui)).is_ok());
    h.prefs.seed(
        &id("a"),
        ClusterPrefs {
            read_only: true,
            exec_in_read_only: false,
            ..ClusterPrefs::default()
        },
    );
    assert!(h.dispatch(shell(None), ctx(Initiator::Ui)).is_err());
}

#[test]
fn a_writable_cluster_opens_a_shell_without_a_confirmation() {
    let h = Harness::with_every_command();
    h.connect("a", false);
    for id_ in EXEC {
        let command = crate::testing_posture::sample(id_, "a");
        let out = h.dispatch(command, ctx(Initiator::Command)).unwrap();
        assert!(
            matches!(out, Outcome::Completed(_)),
            "{id_} must run at once: {out:?}"
        );
    }
    assert_eq!(h.bus.guard().pending_confirmations(), 0);
}

#[test]
fn every_open_is_audited_with_initiator_target_and_container_but_no_content() {
    let h = Harness::with_every_command();
    h.connect("a", false);
    h.dispatch(shell(Some("app")), ctx(Initiator::Ui)).unwrap();
    h.dispatch(shell(None), ctx(Initiator::Command)).unwrap();
    h.dispatch(
        Command::PodAttach {
            target: pod("a", "web-0"),
            container: Some("sidecar".into()),
        },
        ctx(Initiator::Ui),
    )
    .unwrap();
    // The arguments of an exec may be a password or a query: only the program is recorded.
    h.dispatch(
        exec(&["psql", "-c", "select * from users where token = 'hunter2'"]),
        ctx(Initiator::Ui),
    )
    .unwrap();

    let audit = h.audit();
    let seen: Vec<_> = audit
        .iter()
        .map(|r| {
            (
                r.cmd.as_ref(),
                r.initiator,
                r.detail.as_deref().unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            ("pod::Shell", Initiator::Ui, "session=shell container=app"),
            (
                "pod::Shell",
                Initiator::Command,
                "session=shell container=(default)"
            ),
            (
                "pod::Attach",
                Initiator::Ui,
                "session=attach container=sidecar"
            ),
            (
                "pod::Exec",
                Initiator::Ui,
                "session=exec container=app program=psql"
            ),
        ]
    );
    for record in &audit {
        assert_eq!(record.target, pod("a", "web-0"));
        assert_eq!(record.cluster, id("a"));
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
        let json = serde_json::to_string(record).unwrap();
        assert!(
            !json.contains("hunter2") && !json.contains("select"),
            "{json}"
        );
    }
}

#[test]
fn a_failing_handler_is_audited_as_failed() {
    let h = Harness::with_extra(|reg, _, _| {
        reg.register(
            crate::testing::declared(CommandId::POD_SHELL),
            |_: Command, _: crate::command_bus::HandlerContext| async {
                Err::<crate::command_bus::CommandOutput, _>(OxiError::internal(
                    "the window is gone",
                ))
            },
        )
    });
    h.connect("a", false);
    let err = h.dispatch(shell(None), ctx(Initiator::Ui)).unwrap_err();
    assert!(matches!(err, DispatchError::Handler(_)), "{err:?}");
    assert_eq!(h.outcomes(), [(AuditOutcome::Failed, Initiator::Ui)]);
}

#[test]
fn a_cluster_that_is_not_open_or_not_connected_refuses_and_says_so() {
    let h = Harness::with_every_command();
    let err = h.dispatch(shell(None), ctx(Initiator::Ui)).unwrap_err();
    assert!(matches!(err, DispatchError::NoSession(_)), "{err:?}");
    // Open but never connected: nothing to open a session on.
    h.manager.open(&cluster_context("a"), Default::default());
    let err = h.dispatch(shell(None), ctx(Initiator::Ui)).unwrap_err();
    assert!(matches!(err, DispatchError::NotConnected { .. }), "{err:?}");
    assert!(h.calls().is_empty());
    let outcomes: Vec<_> = h.audit().iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes, [AuditOutcome::Denied, AuditOutcome::Failed]);
}

#[test]
fn an_unwritable_audit_log_refuses_the_next_open() {
    let h = Harness::with_every_command();
    h.connect("a", false);
    // The record of this open cannot be written: the caller is told.
    h.state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
    let err = h.dispatch(shell(None), ctx(Initiator::Ui)).unwrap_err();
    assert!(matches!(err, DispatchError::AuditFailed(_)), "{err:?}");
    assert_eq!(h.calls().len(), 1);
    // While the log stays down nothing opens: the backlog must land first, as for a mutation.
    h.state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
    let err = h.dispatch(shell(None), ctx(Initiator::Ui)).unwrap_err();
    assert!(matches!(err, DispatchError::AuditUnavailable(_)), "{err:?}");
    assert_eq!(h.calls().len(), 1, "nothing opened without an audit trail");
    // Back up: the backlog lands, then the open runs.
    assert!(h.dispatch(shell(None), ctx(Initiator::Ui)).is_ok());
    assert_eq!(h.audit().len(), 2, "the first record was kept and written");
}

#[test]
fn an_agent_is_held_to_the_same_policy() {
    let h = read_only_with(false);
    let agent = DispatchContext::new(Initiator::Agent, "claude");
    let err = h.dispatch(shell(None), agent).unwrap_err();
    assert!(matches!(err, DispatchError::ReadOnly { .. }), "{err:?}");
}

#[test]
fn the_exec_tool_stubs_are_unsafe_interactive_and_hidden_from_agents_by_default() {
    let h = Harness::with_every_command();
    for id_ in EXEC {
        let tool = h.bus.tool(id_).expect("every exec command has a stub");
        assert!(tool.name.as_str().starts_with("k8s.pod_"), "{}", tool.name);
        assert!(
            tool.annotations.unsafe_ && tool.annotations.interactive,
            "{}",
            tool.name
        );
        assert!(tool.annotations.agent_hidden, "{}", tool.name);
        assert_eq!(tool.risk, Some(oxikube_domain::Risk::High), "{}", tool.name);
        assert!(!tool.read_only_hint());
    }
    assert_eq!(
        h.bus.tool(CommandId::POD_EXEC).unwrap().name.as_str(),
        "k8s.pod_exec"
    );

    let default_set: Vec<_> = h
        .bus
        .agent_tools(false)
        .map(|t| t.name.to_string())
        .collect();
    for hidden in [
        "k8s.pod_exec",
        "k8s.pod_shell",
        "k8s.pod_attach",
        // A mutation that ends in a terminal (E09-S10): hidden the same way.
        "k8s.pod_debug",
    ] {
        assert!(
            !default_set.iter().any(|n| n == hidden),
            "{hidden} is hidden by default"
        );
    }
    assert!(
        default_set.iter().any(|n| n == "k8s.pod_view_logs"),
        "other tools stay"
    );
    let opted_in: Vec<_> = h
        .bus
        .agent_tools(true)
        .map(|t| t.name.to_string())
        .collect();
    assert!(opted_in.iter().any(|n| n == "k8s.pod_exec"));
    assert!(opted_in.iter().any(|n| n == "k8s.pod_debug"));
    assert_eq!(opted_in.len(), default_set.len() + EXEC.len() + 1);
}
