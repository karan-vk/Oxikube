//! Registration per crate `init`: duplicates, undeclared ids, re-described metadata and
//! the MCP tool stub of every registered command.

use oxikube_domain::Capabilities;
use oxikube_domain::command::{CommandId, CommandMeta, CommandScope};
use oxikube_domain::safety::Risk;
use oxikube_ports::ToolName;

use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::testing::{Calls, Harness, MUTATING, READS, declared, register_mutations};
use oxikube_domain::command::Command;

fn noop(
    _: Command,
    _: HandlerContext,
) -> futures::future::Ready<oxikube_domain::OxiResult<CommandOutput>> {
    futures::future::ready(Ok(CommandOutput::none()))
}

#[test]
fn crates_register_through_install_and_the_owner_is_recorded() {
    let h = Harness::new();
    for id in MUTATING {
        assert_eq!(h.bus.owner(id), Some("test_workloads"), "{id}");
    }
    for id in READS {
        assert_eq!(h.bus.owner(id), Some("test_views"), "{id}");
    }
    for id in [
        CommandId::CLUSTER_TOGGLE_READ_ONLY,
        CommandId::CLUSTER_SET_COLOUR,
        CommandId::CLUSTER_APPLY_PRESET,
    ] {
        assert_eq!(h.bus.owner(id), Some("oxikube_app::posture"), "{id}");
    }
    assert!(!h.bus.is_registered(CommandId::APP_QUIT));
    let palette: Vec<_> = h.bus.commands().map(|m| m.id).collect();
    assert_eq!(palette.len(), MUTATING.len() + READS.len() + 3);
}

#[test]
fn duplicate_registration_is_rejected_naming_both_crates() {
    let calls = Calls::default();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_resources_ui", |reg| {
            register_mutations(reg, &calls)
        })
        .unwrap();
    let err = registry
        .install("oxikube_workloads_ui", |reg| {
            reg.register(declared(CommandId::POD_DELETE), noop)
        })
        .unwrap_err();
    match err {
        RegisterError::Duplicate { id, first, second } => {
            assert_eq!(id, CommandId::POD_DELETE);
            assert_eq!(first, "oxikube_resources_ui");
            assert_eq!(second, "oxikube_workloads_ui");
        }
        other => panic!("expected Duplicate, got {other:?}"),
    }
    assert_eq!(
        registry.len(),
        MUTATING.len(),
        "the duplicate left nothing behind"
    );

    let mut bare = CommandRegistry::new();
    bare.register(declared(CommandId::APP_QUIT), noop).unwrap();
    let err = bare
        .register(declared(CommandId::APP_QUIT), noop)
        .unwrap_err();
    assert!(err.to_string().contains("registered twice"), "{err}");
}

#[test]
fn undeclared_ids_are_rejected() {
    let mut registry = CommandRegistry::new();
    let meta = CommandMeta::read(
        CommandId::new("test::Undeclared"),
        "Nope",
        CommandScope::Global,
        Capabilities::empty(),
    );
    let err = registry.register(meta, noop).unwrap_err();
    assert!(matches!(err, RegisterError::Undeclared(id) if id.as_str() == "test::Undeclared"));
    assert!(registry.is_empty());
}

#[test]
fn metadata_cannot_be_redescribed_to_weaken_the_guard() {
    let mut registry = CommandRegistry::new();
    let declared_delete = declared(CommandId::POD_DELETE);
    let as_read = CommandMeta::read(
        CommandId::POD_DELETE,
        declared_delete.title,
        declared_delete.scope,
        Capabilities::empty(),
    );
    let lower_risk = CommandMeta::mutation(
        CommandId::POD_DELETE,
        declared_delete.title,
        declared_delete.scope,
        Risk::Low,
        Capabilities::empty(),
    );
    for meta in [as_read, lower_risk] {
        let err = registry.register(meta, noop).unwrap_err();
        assert!(
            matches!(err, RegisterError::MetaMismatch(CommandId::POD_DELETE)),
            "{err:?}"
        );
    }
    assert!(!registry.contains(CommandId::POD_DELETE));
}

#[test]
fn every_registered_command_has_a_tool_stub() {
    let h = Harness::new();
    let mut stubs = 0;
    for meta in h.bus.commands() {
        if meta.privileged {
            assert!(
                h.bus.tool(meta.id).is_none(),
                "{} is privileged: no tool",
                meta.id
            );
            continue;
        }
        let tool = h
            .bus
            .tool(meta.id)
            .unwrap_or_else(|| panic!("{} has no tool stub", meta.id));
        stubs += 1;
        assert_eq!(tool.name, ToolName::new(&meta.id.tool_name()).unwrap());
        assert_eq!(tool.title.as_deref(), Some(meta.title));
        assert_eq!(tool.risk, meta.risk, "{}", meta.id);
        assert_eq!(tool.is_mutating(), meta.mutating, "{}", meta.id);
        assert_eq!(tool.needs, meta.needs);
        assert!(tool.description.contains(meta.id.as_str()));
        tool.validate().unwrap();
    }
    // The posture commands add two stubs (colour, preset); the privileged toggle has none.
    assert_eq!(stubs, MUTATING.len() + READS.len() + 2);
    assert_eq!(h.bus.tools().count(), stubs);
    assert_eq!(
        h.bus.tool(CommandId::POD_DELETE).unwrap().name.as_str(),
        "k8s.pod_delete"
    );
}

#[test]
fn every_declared_command_can_be_registered_with_a_stub() {
    let mut registry = CommandRegistry::new();
    for meta in oxikube_domain::command::COMMANDS {
        registry
            .register(*meta, noop)
            .unwrap_or_else(|e| panic!("{}: {e}", meta.id));
    }
    assert_eq!(registry.len(), oxikube_domain::command::COMMANDS.len());
}
