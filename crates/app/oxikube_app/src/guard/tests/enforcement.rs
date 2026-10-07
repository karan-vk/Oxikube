//! Read-only enforcement through the [`CommandBus`](crate::CommandBus), over the whole registry.
//!
//! The tests iterate every declared command (the harness registers a recording handler for each
//! one), so a mutating command added later is covered the day it is declared: it either has a
//! sample in `testing_posture::sample` and is denied here, or this suite fails and says so.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{COMMANDS, Command, CommandId};
use oxikube_domain::{ErrorKind, OxiError};
use serde_json::Value;

use crate::command_bus::{CommandOutput, DispatchContext, DispatchError, HandlerContext, Outcome};
use crate::testing::{Harness, ctx, declared, id};
use crate::testing_posture::sample;

/// The commands the posture pipeline owns: not mutations, tested in `posture.rs`.
fn is_posture(meta: &oxikube_domain::CommandMeta) -> bool {
    meta.privileged || crate::guard::policy::is_posture(&sample(meta.id, "a"))
}

fn mutating() -> Vec<CommandId> {
    COMMANDS
        .iter()
        .filter(|m| m.mutating)
        .map(|m| m.id)
        .collect()
}

#[test]
fn the_harness_registers_every_declared_command() {
    let h = Harness::with_every_command();
    for meta in COMMANDS {
        assert!(
            h.bus.is_registered(meta.id),
            "{} is not registered",
            meta.id
        );
    }
    assert!(!mutating().is_empty());
}

#[test]
fn a_read_only_cluster_denies_every_mutating_command_for_every_initiator() {
    let h = Harness::with_every_command();
    h.connect("a", true);
    let ids = mutating();
    for command in &ids {
        for initiator in Initiator::ALL {
            let err = h
                .dispatch(sample(*command, "a"), ctx(initiator))
                .expect_err(&format!("{command} must be denied for {initiator}"));
            match &err {
                DispatchError::ReadOnly { cluster, context } => {
                    assert_eq!(cluster, &id("a"));
                    assert_eq!(context.as_str(), "a", "the error names the cluster");
                }
                other => panic!("{command} as {initiator}: expected ReadOnly, got {other:?}"),
            }
            assert_eq!(OxiError::from(err).kind(), ErrorKind::Forbidden);
        }
    }
    assert!(h.writes("a").is_empty(), "no request reached the cluster");
    assert!(
        h.calls().is_empty(),
        "no handler ran; denial comes before confirmation and execution"
    );
    let audit = h.audit();
    assert_eq!(audit.len(), ids.len() * Initiator::ALL.len());
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));
    assert_eq!(h.bus.guard().pending_confirmations(), 0, "never asks first");
}

#[test]
fn the_same_commands_reach_the_guard_on_a_writable_cluster() {
    // The control for the test above: with the flag off, nothing is denied for being
    // read-only, so the denial really is the flag and not a missing sample or handler.
    let h = Harness::with_every_command();
    h.connect("a", false);
    for command in mutating() {
        let out = h.dispatch(sample(command, "a"), ctx(Initiator::Agent));
        assert!(
            matches!(out, Ok(Outcome::NeedsConfirmation(_))),
            "{command}: {out:?}"
        );
    }
}

#[test]
fn read_commands_still_run_on_a_read_only_cluster() {
    let h = Harness::with_every_command();
    h.connect("a", true);
    let reads: Vec<_> = COMMANDS
        .iter()
        .filter(|m| !m.mutating && !m.exec && !is_posture(m))
        .collect();
    assert!(reads.len() >= 10, "{} read commands", reads.len());
    for meta in &reads {
        for initiator in Initiator::ALL {
            let out = h.dispatch(sample(meta.id, "a"), ctx(initiator));
            assert!(
                matches!(out, Ok(Outcome::Completed(_))),
                "{}: {out:?}",
                meta.id
            );
        }
    }
    assert!(h.audit().is_empty(), "reads are not audited");
}

/// What an MCP server does with a `tools/call`: the tool name picks the command, the arguments
/// are its payload, and the dispatch is `Initiator::Agent`.
fn tool_call(tool: &str, arguments: Value) -> Command {
    let meta = COMMANDS
        .iter()
        .find(|m| m.id.tool_name() == tool)
        .unwrap_or_else(|| panic!("no command for tool {tool}"));
    let mut value = arguments;
    value["type"] = Value::from(meta.id.as_str());
    serde_json::from_value(value).expect("tool arguments are the command payload")
}

#[test]
fn mcp_mutation_tools_are_denied_on_a_read_only_cluster() {
    let h = Harness::with_every_command();
    h.connect("a", true);
    // The exec tools carry a risk too but are not mutations: `exec.rs` covers them. `pod::Debug`
    // and `node::Shell` are interactive and mutations, so they stay in this list.
    let exec_tool = |name: &str| COMMANDS.iter().any(|m| m.exec && m.id.tool_name() == name);
    let tools: Vec<_> = h
        .bus
        .tools()
        .filter(|t| t.is_mutating() && !exec_tool(t.name.as_str()))
        .collect();
    assert_eq!(
        tools.len(),
        mutating().len(),
        "every mutation has a tool stub"
    );
    for tool in tools {
        let meta = COMMANDS
            .iter()
            .find(|m| m.id.tool_name() == tool.name.as_str())
            .unwrap();
        let mut arguments = serde_json::to_value(sample(meta.id, "a")).unwrap();
        arguments.as_object_mut().unwrap().remove("type");
        let command = tool_call(tool.name.as_str(), arguments);
        let agent = DispatchContext::new(Initiator::Agent, "claude");
        let err = h.dispatch(command, agent).unwrap_err();
        assert!(
            matches!(err, DispatchError::ReadOnly { .. }),
            "{}: {err:?}",
            tool.name
        );
    }
    assert!(h.writes("a").is_empty());
    assert!(h.audit().iter().all(|r| r.initiator == Initiator::Agent));
}

#[test]
fn an_agent_stub_mutation_tool_is_denied_through_the_same_path() {
    // The stub E26 will replace: a mutation tool registered by another crate under its own
    // owner name reaches the guard like any command, so read-only mode blocks it with no
    // extra wiring.
    let h = Harness::with_extra(|reg, calls, _| {
        let calls = calls.clone();
        reg.install("oxikube_mcp_stub", |reg| {
            reg.register(
                declared(CommandId::NODE_CORDON),
                move |cmd: Command, cx: HandlerContext| {
                    calls.lock().push(crate::testing::Call {
                        id: cmd.id(),
                        initiator: cx.initiator(),
                        cluster: cx.cluster().cloned(),
                        mutation: cx.mutation().is_some(),
                        dry_run: false,
                    });
                    async { Ok(CommandOutput::none()) }
                },
            )
        })
    });
    h.connect("a", true);
    let tool = h.bus.tool(CommandId::NODE_CORDON).expect("stub tool");
    assert_eq!(tool.name.as_str(), "k8s.node_cordon");
    let command = tool_call(
        "k8s.node_cordon",
        serde_json::json!({
            "target": sample(CommandId::NODE_CORDON, "a").target().unwrap(),
        }),
    );
    let err = h
        .dispatch(command, DispatchContext::new(Initiator::Agent, "claude"))
        .unwrap_err();
    assert_eq!(err.read_only_cluster(), Some(&id("a")));
    assert!(h.calls().is_empty());
}

#[test]
fn read_only_going_on_during_a_running_command_stops_its_next_write() {
    // The second check: the guard admitted the command while the cluster was writable, and
    // read-only mode went on before the handler's second request. That request never leaves.
    let h = Harness::with_extra(|reg, _, manager| {
        let manager = manager.clone();
        reg.register(
            declared(CommandId::NODE_CORDON),
            move |cmd: Command, cx: HandlerContext| {
                let manager = manager.clone();
                async move {
                    let mutation = cx.require_mutation()?;
                    let target = cmd.target().expect("target");
                    let options = mutation.delete_options();
                    let delete = || {
                        mutation.writer().delete(
                            &target.gvk,
                            target.namespace(),
                            &target.name,
                            &options,
                        )
                    };
                    delete().await?;
                    manager.set_read_only(&id("a"), true)?;
                    delete().await?;
                    Ok(CommandOutput::none())
                }
            },
        )
    });
    h.connect("a", false);
    h.allow_deletes("a", 2);
    let cordon = sample(CommandId::NODE_CORDON, "a");
    let err = h.confirm_and_run(cordon, ctx(Initiator::Ui)).unwrap_err();
    match err {
        DispatchError::Handler(e) => {
            assert_eq!(e.kind(), ErrorKind::Forbidden);
            assert!(e.message().contains("read-only"), "{}", e.message());
        }
        other => panic!("expected the handler's Forbidden error, got {other:?}"),
    }
    assert_eq!(h.writes("a").len(), 1, "only the first request was sent");
    assert_eq!(h.outcomes(), [(AuditOutcome::Failed, Initiator::Ui)]);
}
