//! [`ToolRegistry`]: the tools hosted agents can call, by name.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxikube_domain::{Capabilities, OxiError, OxiResult};
use oxikube_ports::{ToolContext, ToolDef, ToolName, ToolOutput, ToolPort};
use parking_lot::RwLock;
use serde_json::Value;

use super::schema::validate_args;

/// Why a tool was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterToolError {
    /// The definition breaks an invariant ([`ToolDef::validate`]).
    #[error("tool definition rejected: {0}")]
    Invalid(String),
    /// The tool changes cluster state. Such a tool is only ever run through `MutationGuard` with a
    /// permission prompt (ADR 0008, 0012); this registry has no guard in reach, so it holds
    /// read-only tools only until the guarded registration of the agent epic (E26) exists.
    #[error("tool {0} is mutating: it cannot be registered without MutationGuard")]
    Mutating(ToolName),
    /// A tool of that name is registered.
    #[error("tool {0} is already registered")]
    Duplicate(ToolName),
}

/// The tools exposed to agents, by [`ToolName`]. Every feature registers its tools as it lands
/// (non-negotiable 4); the MCP server (`oxikube_mcp`) lists them and calls them through
/// [`invoke`](Self::invoke). One per app.
///
/// This holds read-only tools: [`register`](Self::register) refuses a tool that changes cluster
/// state, which belongs behind `MutationGuard` (E26). A tool's [`needs`](ToolDef::needs) are the
/// capabilities a session must have: [`visible`](Self::visible) hides the tool otherwise, and the
/// permission policy of E26 sits on top of that.
#[derive(Default)]
pub struct ToolRegistry {
    tools: RwLock<BTreeMap<ToolName, Arc<dyn ToolPort>>>,
}

impl ToolRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `tool` under the name of its definition.
    ///
    /// # Errors
    ///
    /// [`RegisterToolError`] for an invalid definition, a mutating tool, or a name in use.
    pub fn register(&self, tool: Arc<dyn ToolPort>) -> Result<(), RegisterToolError> {
        let def = tool.def();
        def.validate()
            .map_err(|e| RegisterToolError::Invalid(e.message().to_owned()))?;
        if def.is_mutating() {
            return Err(RegisterToolError::Mutating(def.name.clone()));
        }
        let name = def.name.clone();
        let mut tools = self.tools.write();
        if tools.contains_key(&name) {
            return Err(RegisterToolError::Duplicate(name));
        }
        tools.insert(name, tool);
        Ok(())
    }

    /// The tool named `name`.
    pub fn get(&self, name: &str) -> Option<Arc<dyn ToolPort>> {
        let name = ToolName::new(name).ok()?;
        self.tools.read().get(&name).cloned()
    }

    /// Every definition, sorted by name.
    pub fn defs(&self) -> Vec<ToolDef> {
        self.tools
            .read()
            .values()
            .map(|t| t.def().clone())
            .collect()
    }

    /// The definitions of the tools a session with `capabilities` may use and agents are offered
    /// by default, sorted by name: a tool hidden from agents ([`ToolDef::agent_exposed_by_default`],
    /// the unsafe interactive ones) is left out.
    pub fn visible(&self, capabilities: Capabilities) -> Vec<ToolDef> {
        self.visible_with_hidden(capabilities, false)
    }

    /// [`visible`](Self::visible), with the tools hidden from agents too when `include_hidden`
    /// (the user turned them on in the settings).
    pub fn visible_with_hidden(
        &self,
        capabilities: Capabilities,
        include_hidden: bool,
    ) -> Vec<ToolDef> {
        self.tools
            .read()
            .values()
            .map(|t| t.def())
            .filter(|def| capabilities.contains(def.needs))
            .filter(|def| include_hidden || def.agent_exposed_by_default())
            .cloned()
            .collect()
    }

    /// Runs the tool `name` with `args`, after checking them against its input schema.
    ///
    /// # Errors
    ///
    /// A failure of the call itself: [`NotFound`](oxikube_domain::ErrorKind::NotFound) for an
    /// unknown tool, a validation error for arguments that break the schema, and whatever the tool
    /// returns as `Err`. A tool that ran and failed is `Ok` with [`ToolOutput::is_error`] set.
    pub async fn invoke(
        &self,
        name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> OxiResult<ToolOutput> {
        let tool = self
            .get(name)
            .ok_or_else(|| OxiError::not_found(format!("unknown tool {name:?}")))?;
        validate_args(&tool.def().input_schema, &args)?;
        tool.invoke(args, ctx).await
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.tools.read().keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use futures::executor::block_on;
    use oxikube_domain::{ErrorKind, Risk};
    use serde_json::json;

    use super::*;

    struct Echo(ToolDef);

    #[async_trait]
    impl ToolPort for Echo {
        fn def(&self) -> &ToolDef {
            &self.0
        }
        async fn invoke(&self, args: Value, _: &ToolContext) -> OxiResult<ToolOutput> {
            Ok(ToolOutput::json(args))
        }
    }

    fn schema() -> Value {
        json!({"type": "object", "properties": {"x": {"type": "integer"}}, "additionalProperties": false})
    }

    fn echo(name: &str) -> Arc<Echo> {
        Arc::new(Echo(ToolDef::read_only(
            ToolName::new(name).unwrap(),
            "echoes",
            schema(),
        )))
    }

    #[test]
    fn a_registered_tool_is_listed_and_invoked_after_its_schema_check() {
        let registry = ToolRegistry::new();
        registry.register(echo("app.echo")).unwrap();
        assert_eq!(registry.defs().len(), 1);
        let ctx = ToolContext::agent();
        let out = block_on(registry.invoke("app.echo", json!({"x": 1}), &ctx)).unwrap();
        assert_eq!(out.structured, Some(json!({"x": 1})));
        let err = block_on(registry.invoke("app.echo", json!({"x": "no"}), &ctx)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation);
        let err = block_on(registry.invoke("app.missing", json!({}), &ctx)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn names_are_unique_and_invalid_or_mutating_tools_are_refused() {
        let registry = ToolRegistry::new();
        registry.register(echo("app.echo")).unwrap();
        assert!(matches!(
            registry.register(echo("app.echo")),
            Err(RegisterToolError::Duplicate(_))
        ));
        let blank = Echo(ToolDef::read_only(
            ToolName::new("app.blank").unwrap(),
            " ",
            schema(),
        ));
        assert!(matches!(
            registry.register(Arc::new(blank)),
            Err(RegisterToolError::Invalid(_))
        ));
        let mutating = Echo(ToolDef::mutating(
            ToolName::new("k8s.pod_delete").unwrap(),
            "deletes",
            schema(),
            Risk::High,
        ));
        assert!(matches!(
            registry.register(Arc::new(mutating)),
            Err(RegisterToolError::Mutating(_))
        ));
    }

    #[test]
    fn visibility_follows_the_capabilities_a_tool_needs() {
        let registry = ToolRegistry::new();
        registry.register(echo("app.open")).unwrap();
        registry
            .register(Arc::new(Echo(
                ToolDef::read_only(
                    ToolName::new("k8s.logs_only").unwrap(),
                    "needs logs",
                    schema(),
                )
                .with_needs(Capabilities::LOGS),
            )))
            .unwrap();
        assert_eq!(registry.visible(Capabilities::empty()).len(), 1);
        assert_eq!(registry.visible(Capabilities::LOGS).len(), 2);
    }

    #[test]
    fn a_tool_hidden_from_agents_is_listed_only_when_asked_for() {
        let registry = ToolRegistry::new();
        registry.register(echo("app.open")).unwrap();
        let hidden = ToolDef::read_only(
            ToolName::new("app.hidden").unwrap(),
            "hidden by default",
            schema(),
        )
        .with_annotations(oxikube_ports::ToolAnnotations {
            agent_hidden: true,
            ..Default::default()
        });
        registry.register(Arc::new(Echo(hidden))).unwrap();
        let names = |defs: Vec<ToolDef>| -> Vec<String> {
            defs.iter().map(|d| d.name.to_string()).collect()
        };
        assert_eq!(names(registry.visible(Capabilities::all())), ["app.open"]);
        assert_eq!(
            names(registry.visible_with_hidden(Capabilities::all(), true)),
            ["app.hidden", "app.open"]
        );
        assert_eq!(registry.defs().len(), 2, "registered, just not offered");
    }
}
