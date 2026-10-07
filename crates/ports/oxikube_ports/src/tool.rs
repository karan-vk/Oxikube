//! Tools: [`ToolDef`] and [`ToolPort`], shaped after MCP tool semantics.
//!
//! A tool is what an agent can call over MCP: a name, a description, a JSON
//! Schema for its arguments, a result made of content parts, and hints about
//! its effect. `oxikube_app::ToolRegistry` holds the tools, `oxikube_mcp`
//! exposes them over stdio, and every feature registers its tools as it lands
//! (non-negotiable 4).
//!
//! | MCP `Tool` field | Here |
//! |---|---|
//! | `name` | [`ToolDef::name`] ([`ToolName`], validated) |
//! | `title` | [`ToolDef::title`] |
//! | `description` | [`ToolDef::description`] |
//! | `inputSchema` | [`ToolDef::input_schema`] |
//! | `outputSchema` | [`ToolDef::output_schema`] |
//! | `annotations.readOnlyHint` | [`ToolDef::read_only_hint`] (derived: no risk) |
//! | `annotations.destructiveHint` | [`ToolDef::destructive_hint`] (derived: risk at least high) |
//! | `annotations.idempotentHint` | [`ToolAnnotations::idempotent`] |
//! | `annotations.openWorldHint` | [`ToolAnnotations::open_world`] |
//! | (ours) interactive, unsafe, hidden from agents | [`ToolAnnotations::interactive`], [`unsafe_`](ToolAnnotations::unsafe_), [`agent_hidden`](ToolAnnotations::agent_hidden) |
//! | `icons`, `_meta` | deferred (no UI uses them) |
//! | `CallToolResult.content` | [`ToolOutput::content`] |
//! | `CallToolResult.structuredContent` | [`ToolOutput::structured`] |
//! | `CallToolResult.isError` | [`ToolOutput::is_error`] |
//!
//! # Risk, not just hints
//!
//! MCP hints are advisory and may come from untrusted servers. Ours are
//! authoritative: [`ToolDef::risk`] is `Some` exactly when the tool changes
//! cluster state, and the registry gates on it. A mutating tool is only ever
//! invoked through `MutationGuard` with
//! [`Initiator::Agent`] plus a permission
//! prompt (ADR 0008, 0012); `ToolPort::invoke` itself does not check.
//!
//! # Errors
//!
//! Two kinds, as in MCP. A *tool execution* failure the model should see and
//! react to (pod not found, forbidden) is `Ok(ToolOutput::error(..))`. A
//! failure of the call itself (unknown tool, arguments that violate the schema,
//! the guard refusing) is `Err(OxiError)` and becomes a protocol error.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::agent::ContextBlock;
use oxikube_domain::command::CommandMeta;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{Capabilities, Initiator, OxiError, OxiResult, Risk};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::{AgentSessionId, ToolCallId};
use crate::context::ContentPart;

/// A tool name: dot-separated lowercase segments such as `k8s.pod_delete`,
/// `helm.rollback` or `ext.my_plugin.lint`.
///
/// Each segment is `[a-z][a-z0-9_]*`; there are at least two segments, and an
/// `ext.<id>.<name>` name has at least three. Command-derived names come from
/// [`CommandId::tool_name`](oxikube_domain::command::CommandId::tool_name):
/// `k8s.*` for resource verbs, `helm.*`, `argo.*`, `app.*` for UI-level
/// commands.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ToolName(Arc<str>);

impl ToolName {
    /// Whether `s` is a well-formed tool name.
    pub fn is_well_formed(s: &str) -> bool {
        let mut segments = 0usize;
        for seg in s.split('.') {
            let b = seg.as_bytes();
            let ok = !b.is_empty()
                && b[0].is_ascii_lowercase()
                && b.iter()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_');
            if !ok {
                return false;
            }
            segments += 1;
        }
        let min = if s.starts_with("ext.") { 3 } else { 2 };
        segments >= min
    }

    /// Validates and wraps `s`.
    pub fn new(s: &str) -> OxiResult<Self> {
        if Self::is_well_formed(s) {
            Ok(Self(Arc::from(s)))
        } else {
            Err(OxiError::validation(format!(
                "invalid tool name {s:?}: expected dot-separated [a-z][a-z0-9_]* segments (ext.<id>.<name> for extensions)"
            )))
        }
    }

    /// The name text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first segment (`k8s`, `helm`, `argo`, `app`, `ext`, ...).
    pub fn namespace(&self) -> &str {
        self.0.split('.').next().unwrap_or_default()
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ToolName {
    type Error = OxiError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<ToolName> for String {
    fn from(value: ToolName) -> Self {
        value.0.to_string()
    }
}

/// Behavioural hints that do not come from the risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ToolAnnotations {
    /// Calling again with the same arguments has no further effect.
    #[serde(default)]
    pub idempotent: bool,
    /// The tool reaches outside the cluster (the network, a registry).
    /// Oxikube's own tools are closed-world, so this defaults to `false`
    /// (the MCP default is `true`).
    #[serde(default)]
    pub open_world: bool,
    /// The tool opens an interactive session (a shell, an attach): its effect is whatever the
    /// user types next, so it cannot be described by a schema, and an agent can only start it
    /// (E09-S08, `k8s.pod_shell`).
    #[serde(default)]
    pub interactive: bool,
    /// The tool is unsafe to run unattended: it gives access no policy can bound (a shell in a
    /// container). Shown with the tool, and a reason for [`agent_hidden`](Self::agent_hidden).
    #[serde(default, rename = "unsafe")]
    pub unsafe_: bool,
    /// Not offered to agents unless the user turns it on in the settings. The default (`false`)
    /// offers the tool; an `unsafe_` tool sets this.
    #[serde(default)]
    pub agent_hidden: bool,
}

/// The static description of a tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    /// Unique name.
    pub name: ToolName,
    /// Display title; falls back to the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// What the tool does, written for the model.
    pub description: String,
    /// JSON Schema (an object schema) for the arguments.
    pub input_schema: Value,
    /// JSON Schema for [`ToolOutput::structured`], when the tool returns it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Blast radius: `Some` exactly when the tool changes cluster state.
    /// Drives the confirmation tier and the permission prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<Risk>,
    /// Capabilities the session must have; the registry hides the tool
    /// otherwise. A mutating tool needs [`Capabilities::MUTATE`].
    #[serde(default)]
    pub needs: Capabilities,
    /// Other hints.
    #[serde(default)]
    pub annotations: ToolAnnotations,
}

impl ToolDef {
    /// A read-only tool.
    pub fn read_only(name: ToolName, description: impl Into<String>, input_schema: Value) -> Self {
        Self {
            name,
            title: None,
            description: description.into(),
            input_schema,
            output_schema: None,
            risk: None,
            needs: Capabilities::empty(),
            annotations: ToolAnnotations::default(),
        }
    }

    /// A tool that changes cluster state at `risk`. Adds
    /// [`Capabilities::MUTATE`] to `needs`.
    pub fn mutating(
        name: ToolName,
        description: impl Into<String>,
        input_schema: Value,
        risk: Risk,
    ) -> Self {
        Self {
            risk: Some(risk),
            needs: Capabilities::MUTATE,
            ..Self::read_only(name, description, input_schema)
        }
    }

    /// The tool for a command: name from [`CommandId::tool_name`](oxikube_domain::command::CommandId::tool_name),
    /// risk, capabilities and title from `meta`.
    ///
    /// Fails with a validation error for a *privileged* command (one that
    /// changes the safety posture, such as lifting read-only mode): ADR 0012
    /// keeps those out of reach of agents, so they get no tool.
    pub fn for_command(
        meta: &CommandMeta,
        description: impl Into<String>,
        input_schema: Value,
    ) -> OxiResult<Self> {
        if !meta.allows(Initiator::Agent) {
            return Err(OxiError::validation(format!(
                "command {} is privileged and cannot be exposed as a tool",
                meta.id
            )));
        }
        let mut def = Self::read_only(
            ToolName::new(&meta.id.tool_name())?,
            description,
            input_schema,
        );
        def.title = Some(meta.title.to_owned());
        def.risk = meta.tool_risk();
        def.needs = meta.needs;
        if meta.interactive {
            // A shell in a container, or a debug container with a shell in it: unsafe,
            // interactive, and not exposed to agents until the user asks for it (E09-S08,
            // E09-S10).
            def.annotations.interactive = true;
            def.annotations.unsafe_ = true;
            def.annotations.agent_hidden = true;
        }
        Ok(def)
    }

    /// Sets the display title.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the output schema.
    #[must_use]
    pub fn with_output_schema(mut self, schema: Value) -> Self {
        self.output_schema = Some(schema);
        self
    }

    /// Sets the annotations.
    #[must_use]
    pub fn with_annotations(mut self, annotations: ToolAnnotations) -> Self {
        self.annotations = annotations;
        self
    }

    /// Adds required capabilities.
    #[must_use]
    pub fn with_needs(mut self, needs: Capabilities) -> Self {
        self.needs |= needs;
        self
    }

    /// Whether the tool changes cluster state or opens a session that can (a `risk` is declared),
    /// so it must not run without `MutationGuard` or the exec policy.
    pub const fn is_mutating(&self) -> bool {
        self.risk.is_some()
    }

    /// Whether the tool is offered to agents without the user turning it on: it is not
    /// [`agent_hidden`](ToolAnnotations::agent_hidden).
    pub const fn agent_exposed_by_default(&self) -> bool {
        !self.annotations.agent_hidden
    }

    /// MCP `readOnlyHint`: the tool does not change its environment.
    pub const fn read_only_hint(&self) -> bool {
        self.risk.is_none()
    }

    /// MCP `destructiveHint`: the tool may destroy data (risk of at least
    /// [`Risk::High`]). Meaningful only for mutating tools.
    pub fn destructive_hint(&self) -> bool {
        self.risk.is_some_and(|r| r >= Risk::High)
    }

    /// Checks the invariants the registry relies on: a non-empty description,
    /// object schemas, and `MUTATE` in `needs` for a mutating tool.
    pub fn validate(&self) -> OxiResult<()> {
        let name = &self.name;
        if self.description.trim().is_empty() {
            return Err(OxiError::validation(format!(
                "tool {name} has an empty description"
            )));
        }
        if !is_object_schema(&self.input_schema) {
            return Err(OxiError::validation(format!(
                "tool {name}: input_schema must be a JSON Schema with \"type\": \"object\""
            )));
        }
        if self
            .output_schema
            .as_ref()
            .is_some_and(|s| !is_object_schema(s))
        {
            return Err(OxiError::validation(format!(
                "tool {name}: output_schema must be a JSON Schema with \"type\": \"object\""
            )));
        }
        // An interactive tool (a shell) carries a risk without changing objects: it needs `exec`.
        let needed = if self.annotations.interactive {
            Capabilities::EXEC
        } else {
            Capabilities::MUTATE
        };
        if self.is_mutating() && !self.needs.contains(needed) {
            return Err(OxiError::validation(format!(
                "tool {name} is mutating but does not need the {} capability",
                if self.annotations.interactive {
                    "exec"
                } else {
                    "mutate"
                }
            )));
        }
        Ok(())
    }
}

fn is_object_schema(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("object")
}

/// Who is calling and on what, passed to every [`ToolPort::invoke`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolContext {
    /// Who asked. `Agent` for MCP calls, `Plugin` for extension calls. Recorded
    /// in the audit record of any mutation.
    pub initiator: Initiator,
    /// The cluster the call targets, when the caller named or defaulted one.
    pub cluster: Option<ClusterId>,
    /// The agent session making the call, for audit and thread matching.
    pub agent_session: Option<AgentSessionId>,
    /// The tool call id the agent announced, when known.
    pub tool_call: Option<ToolCallId>,
}

impl ToolContext {
    /// A context for `initiator` with nothing else set.
    pub fn new(initiator: Initiator) -> Self {
        Self {
            initiator,
            cluster: None,
            agent_session: None,
            tool_call: None,
        }
    }

    /// A context for an MCP call from an agent.
    pub fn agent() -> Self {
        Self::new(Initiator::Agent)
    }

    /// Sets the target cluster.
    #[must_use]
    pub fn with_cluster(mut self, cluster: ClusterId) -> Self {
        self.cluster = Some(cluster);
        self
    }

    /// Sets the calling agent session.
    #[must_use]
    pub fn with_agent_session(mut self, session: AgentSessionId) -> Self {
        self.agent_session = Some(session);
        self
    }

    /// Sets the tool call id.
    #[must_use]
    pub fn with_tool_call(mut self, call: ToolCallId) -> Self {
        self.tool_call = Some(call);
        self
    }
}

/// The result of a tool call.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolOutput {
    /// What the model reads.
    pub content: Vec<ContentPart>,
    /// The same result as JSON matching [`ToolDef::output_schema`], when the
    /// tool has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<Value>,
    /// The tool ran and failed; `content` explains. See the module docs.
    #[serde(default)]
    pub is_error: bool,
}

impl ToolOutput {
    /// A successful text result.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentPart::text(text)],
            structured: None,
            is_error: false,
        }
    }

    /// A successful structured result; `content` carries the JSON as text so
    /// clients without structured support still see it.
    pub fn json(value: Value) -> Self {
        Self {
            content: vec![ContentPart::text(value.to_string())],
            structured: Some(value),
            is_error: false,
        }
    }

    /// A successful result made of one embedded context block.
    pub fn block(block: ContextBlock) -> Self {
        Self {
            content: vec![ContentPart::Resource(block)],
            structured: None,
            is_error: false,
        }
    }

    /// A failed execution the model should see.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ContentPart::text(message)],
            structured: None,
            is_error: true,
        }
    }
}

/// One callable tool. Implemented per tool (or per group) in `oxikube_app` and
/// by integrations.
///
/// # Effects
///
/// Read-only unless [`ToolDef::is_mutating`] says otherwise, in which case the registry routes
/// the call through `MutationGuard`.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// `Err` is for a failure of the call itself:
/// [`Validation`](oxikube_domain::ErrorKind::Validation) for arguments that pass the schema but
/// are unusable, [`Forbidden`](oxikube_domain::ErrorKind::Forbidden) when the session forbids
/// the tool, [`Internal`](oxikube_domain::ErrorKind::Internal) for bugs. A tool that ran and
/// failed returns `Ok(ToolOutput::error(..))`.
#[async_trait]
pub trait ToolPort: Send + Sync {
    /// The tool's static description.
    fn def(&self) -> &ToolDef;

    /// Runs the tool with JSON `args` (validated against
    /// [`ToolDef::input_schema`] by the registry before the call).
    ///
    /// Mutating tools (`def().is_mutating()`) must be reached only through
    /// `MutationGuard`; the registry does that, not this method. Returns
    /// `Ok(ToolOutput::error(..))` for a failed execution and `Err` for a
    /// failure of the call itself (see the module docs).
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_domain::ErrorKind;
    use oxikube_domain::command::{COMMANDS, CommandId};
    use proptest::prelude::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]})
    }

    #[test]
    fn names_follow_the_naming_rules() {
        for ok in [
            "k8s.pod_delete",
            "helm.rollback",
            "argo.sync",
            "app.cluster_toggle_read_only",
            "ext.my_plugin.lint",
        ] {
            assert!(ToolName::is_well_formed(ok), "{ok}");
        }
        for bad in [
            "",
            "k8s",
            "k8s.",
            ".k8s",
            "k8s..x",
            "K8s.pod",
            "k8s.Pod",
            "k8s.pod-delete",
            "k8s.1pod",
            "ext.only",
            "k8s.pod delete",
        ] {
            assert!(!ToolName::is_well_formed(bad), "{bad:?}");
            assert_eq!(
                ToolName::new(bad).unwrap_err().kind(),
                ErrorKind::Validation
            );
        }
        assert_eq!(ToolName::new("helm.rollback").unwrap().namespace(), "helm");
    }

    #[test]
    fn every_registered_command_has_a_valid_tool_name() {
        // Non-negotiable 4: each command maps to a tool. Privileged commands
        // are the one exception and are refused by `for_command`.
        for meta in COMMANDS {
            let tool = ToolDef::for_command(meta, "d", schema());
            if meta.allows(Initiator::Agent) {
                let tool = tool.unwrap_or_else(|e| panic!("{}: {e}", meta.id));
                assert_eq!(tool.name.as_str(), meta.id.tool_name());
                assert_eq!(tool.risk, meta.tool_risk());
                assert_eq!(tool.needs, meta.needs);
                tool.validate().unwrap();
            } else {
                assert_eq!(tool.unwrap_err().kind(), ErrorKind::Validation);
            }
        }
    }

    #[test]
    fn mutating_commands_become_mutating_tools() {
        let meta = COMMANDS
            .iter()
            .find(|m| m.id == CommandId::NODE_DRAIN)
            .unwrap();
        let tool = ToolDef::for_command(meta, "Drain a node", schema()).unwrap();
        assert!(tool.is_mutating());
        assert!(!tool.read_only_hint());
        assert!(tool.destructive_hint());
        assert!(tool.needs.contains(Capabilities::MUTATE));
        assert_eq!(tool.title.as_deref(), Some("Drain Node"));
    }

    #[test]
    fn the_delete_tool_is_advertised_as_destructive() {
        let meta = COMMANDS
            .iter()
            .find(|m| m.id == CommandId::RESOURCE_DELETE)
            .unwrap();
        let tool = ToolDef::for_command(meta, "Delete a resource", schema()).unwrap();
        assert!(tool.destructive_hint());
        assert_eq!(tool.risk, Some(Risk::Irreversible));
    }

    #[test]
    fn read_only_and_mutating_constructors_agree_with_the_hints() {
        let ro = ToolDef::read_only(
            ToolName::new("k8s.pod_list").unwrap(),
            "List pods",
            schema(),
        );
        assert!(!ro.is_mutating() && ro.read_only_hint() && !ro.destructive_hint());
        ro.validate().unwrap();

        let low = ToolDef::mutating(
            ToolName::new("k8s.node_cordon").unwrap(),
            "Cordon",
            schema(),
            Risk::Low,
        );
        assert!(low.is_mutating() && !low.read_only_hint() && !low.destructive_hint());
        assert!(low.needs.contains(Capabilities::MUTATE));
        low.validate().unwrap();
    }

    #[test]
    fn an_exec_command_is_an_unsafe_interactive_tool_hidden_from_agents() {
        for id in [
            CommandId::POD_SHELL,
            CommandId::POD_ATTACH,
            CommandId::POD_EXEC,
        ] {
            let meta = COMMANDS.iter().find(|m| m.id == id).unwrap();
            let tool = ToolDef::for_command(meta, "Open a session", schema()).unwrap();
            tool.validate().unwrap();
            assert_eq!(tool.risk, Some(Risk::High), "{id}");
            assert_eq!(tool.needs, Capabilities::EXEC, "{id}: exec, not mutate");
            assert!(tool.annotations.interactive && tool.annotations.unsafe_);
            assert!(tool.annotations.agent_hidden && !tool.agent_exposed_by_default());
            assert!(!tool.read_only_hint() && tool.destructive_hint());
        }
        // `pod::Debug` is a mutation that ends in a terminal: the same unsafe, interactive,
        // hidden-by-default stub (E09-S10), with the guard's own (low) risk.
        let meta = COMMANDS
            .iter()
            .find(|m| m.id == CommandId::POD_DEBUG)
            .unwrap();
        let debug = ToolDef::for_command(meta, "Add a debug container", schema()).unwrap();
        debug.validate().unwrap();
        assert_eq!(debug.name.as_str(), "k8s.pod_debug");
        assert_eq!(debug.risk, Some(Risk::Low));
        assert!(
            debug
                .needs
                .contains(Capabilities::EXEC | Capabilities::MUTATE)
        );
        assert!(debug.annotations.interactive && debug.annotations.unsafe_);
        assert!(debug.annotations.agent_hidden && !debug.agent_exposed_by_default());
        assert!(!debug.read_only_hint());
        let meta = COMMANDS
            .iter()
            .find(|m| m.id == CommandId::POD_VIEW_LOGS)
            .unwrap();
        let logs = ToolDef::for_command(meta, "Logs", schema()).unwrap();
        assert!(logs.agent_exposed_by_default() && !logs.annotations.interactive);
        let json = serde_json::to_value(ToolAnnotations {
            unsafe_: true,
            ..ToolAnnotations::default()
        })
        .unwrap();
        assert_eq!(json["unsafe"], true, "the wire name is `unsafe`");
    }

    #[test]
    fn validate_rejects_broken_definitions() {
        let name = || ToolName::new("k8s.pod_list").unwrap();
        let err = |def: ToolDef| def.validate().unwrap_err().kind();

        assert_eq!(
            err(ToolDef::read_only(name(), "  ", schema())),
            ErrorKind::Validation
        );
        assert_eq!(
            err(ToolDef::read_only(name(), "d", json!({"type": "string"}))),
            ErrorKind::Validation
        );
        assert_eq!(
            err(ToolDef::read_only(name(), "d", json!(null))),
            ErrorKind::Validation
        );
        assert_eq!(
            err(ToolDef::read_only(name(), "d", schema()).with_output_schema(json!([]))),
            ErrorKind::Validation
        );
        let mut m = ToolDef::mutating(name(), "d", schema(), Risk::Low);
        m.needs = Capabilities::empty();
        assert_eq!(err(m), ErrorKind::Validation);
    }

    #[test]
    fn tool_def_serde_round_trip_and_defaults() {
        let def = ToolDef::mutating(
            ToolName::new("helm.rollback").unwrap(),
            "Roll back a release",
            schema(),
            Risk::High,
        )
        .with_title("Rollback")
        .with_annotations(ToolAnnotations {
            idempotent: true,
            ..ToolAnnotations::default()
        })
        .with_needs(Capabilities::HELM);
        let v = serde_json::to_value(&def).unwrap();
        assert_eq!(v["name"], "helm.rollback");
        assert_eq!(v["risk"], "high");
        assert_eq!(v["needs"], json!(["mutate", "helm"]));
        assert_eq!(serde_json::from_value::<ToolDef>(v).unwrap(), def);

        let minimal: ToolDef = serde_json::from_value(json!({
            "name": "k8s.pod_list", "description": "d", "input_schema": {"type": "object"}
        }))
        .unwrap();
        assert!(!minimal.is_mutating());
        assert!(
            serde_json::from_value::<ToolDef>(json!({
                "name": "Bad Name", "description": "d", "input_schema": {"type": "object"}
            }))
            .is_err()
        );
    }

    #[test]
    fn output_constructors() {
        let ok = ToolOutput::text("done");
        assert!(!ok.is_error && ok.content[0].as_text() == Some("done"));
        let err = ToolOutput::error("pod not found");
        assert!(err.is_error);
        let j = ToolOutput::json(json!({"replicas": 3}));
        assert_eq!(j.structured, Some(json!({"replicas": 3})));
        assert_eq!(j.content[0].as_text(), Some("{\"replicas\":3}"));
        let b = ToolOutput::block(ContextBlock::text("t", "b"));
        assert!(matches!(b.content[0], ContentPart::Resource(_)));
    }

    #[test]
    fn context_builders_carry_audit_fields() {
        let ctx = ToolContext::agent()
            .with_agent_session(AgentSessionId::from("s1"))
            .with_tool_call(ToolCallId::from("c1"));
        assert_eq!(ctx.initiator, Initiator::Agent);
        assert_eq!(ctx.agent_session.unwrap().as_str(), "s1");
        assert_eq!(ctx.tool_call.unwrap().as_str(), "c1");
    }

    proptest! {
        #[test]
        fn arbitrary_names_never_panic_and_round_trip(s in any::<String>()) {
            if let Ok(n) = ToolName::new(&s) {
                prop_assert_eq!(n.as_str(), s.as_str());
                prop_assert!(ToolName::is_well_formed(&s));
            }
        }

        #[test]
        fn generated_well_formed_names_are_accepted(
            segs in proptest::collection::vec("[a-z][a-z0-9_]{0,8}", 2..5)
        ) {
            let name = segs.join(".");
            prop_assume!(!name.starts_with("ext.") || segs.len() >= 3);
            prop_assert!(ToolName::new(&name).is_ok());
        }
    }
}
