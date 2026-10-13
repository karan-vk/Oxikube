//! The registry inventory: the tool, the provider and the command are registered, read-only, and
//! offered to every session.

use oxikube_domain::Capabilities;
use oxikube_domain::command::CommandId;

use super::*;
use crate::tools::events::{GET_EVENTS, InstallError};

#[tokio::test]
async fn the_tool_and_the_provider_are_registered_and_read_only() {
    let h = Harness::new();
    let defs = h.tools.defs();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, [GET_EVENTS]);
    let def = &defs[0];
    assert!(!def.is_mutating());
    assert!(def.risk.is_none(), "no risk: nothing to confirm");
    assert!(def.read_only_hint());
    def.validate().expect("a valid definition");
    assert!(!def.description.is_empty());
    assert_eq!(def.input_schema["additionalProperties"], false);
    assert_eq!(
        h.tools.visible(Capabilities::empty()).len(),
        1,
        "offered to every session, even a read-only cluster's"
    );
    assert_eq!(h.contexts.prefixes(), ["events"]);
}

#[tokio::test]
async fn installing_twice_names_the_taken_tool() {
    let h = Harness::new();
    let again = install(&h.tools, &h.contexts, &h.deps);
    assert!(matches!(again, Err(InstallError::Tool(_))), "{again:?}");
}

#[tokio::test]
async fn the_send_command_has_a_tool_stub_an_agent_may_call() {
    let h = Harness::new();
    let bus = h.bus();
    let id = CommandId::EVENTS_SEND_TO_AGENT;
    assert_eq!(bus.owner(id), Some("oxikube_app::tools::events"));
    let stub = bus.tool(id).expect("a tool stub");
    assert_eq!(stub.name.as_str(), "k8s.events_send_to_agent");
    assert!(!stub.is_mutating());
    assert!(
        bus.agent_tools(false)
            .any(|t| t.name.as_str() == "k8s.events_send_to_agent")
    );
}
