//! Integration fakes: [`FakeIntegrationPort`], [`FakeToolPort`] and
//! [`FakeContextProviderPort`].

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::agent::ContextBlock;
use oxikube_domain::command::CommandMeta;
use oxikube_domain::{Capabilities, OxiResult};
use oxikube_ports::{
    ContextProviderPort, ContextScope, IntegrationPort, IntegrationSession, Mention, SidebarModel,
    ToolContext, ToolDef, ToolOutput, ToolPort,
};
use serde_json::Value;

use crate::script::{CallLog, Script};

// --- IntegrationPort ---------------------------------------------------------------------

/// Queued responses for [`FakeIntegrationPort`].
#[derive(Debug, Default)]
pub struct IntegrationScripts {
    /// `detect`.
    pub detect: Script<Capabilities>,
}

/// One call made on a [`FakeIntegrationPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationCall {
    /// `detect(session)`.
    Detect(IntegrationSession),
    /// `sidebar()`.
    Sidebar,
    /// `commands()`.
    Commands,
    /// `tools()`.
    Tools,
    /// `context_providers()`.
    ContextProviders,
    /// `settings_schema()`.
    SettingsSchema,
}

/// Fake `IntegrationPort` built from configured contributions.
///
/// `detect` pops its script or returns the configured capabilities (empty by default);
/// the other methods return what was configured with the `with_*` builders. Every call,
/// including the synchronous getters, is recorded (`id()` is not).
pub struct FakeIntegrationPort {
    script: IntegrationScripts,
    calls: CallLog<IntegrationCall>,
    id: String,
    capabilities: Capabilities,
    sidebar: SidebarModel,
    commands: Vec<CommandMeta>,
    tools: Vec<Arc<dyn ToolPort>>,
    providers: Vec<Arc<dyn ContextProviderPort>>,
    settings_schema: Value,
}

fake_plumbing!(FakeIntegrationPort, IntegrationScripts, IntegrationCall);

impl std::fmt::Debug for FakeIntegrationPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeIntegrationPort")
            .field("id", &self.id)
            .field("commands", &self.commands.len())
            .field("tools", &self.tools.len())
            .field("providers", &self.providers.len())
            .finish_non_exhaustive()
    }
}

impl FakeIntegrationPort {
    /// An integration called `id` that contributes nothing and detects no capability.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            script: IntegrationScripts::default(),
            calls: CallLog::default(),
            id: id.into(),
            capabilities: Capabilities::empty(),
            sidebar: SidebarModel::default(),
            commands: Vec::new(),
            tools: Vec::new(),
            providers: Vec::new(),
            settings_schema: Value::Null,
        }
    }

    /// Capabilities `detect` returns when nothing is scripted.
    #[must_use]
    pub fn with_capabilities(mut self, capabilities: Capabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// The sidebar contribution.
    #[must_use]
    pub fn with_sidebar(mut self, sidebar: SidebarModel) -> Self {
        self.sidebar = sidebar;
        self
    }

    /// The command contributions.
    #[must_use]
    pub fn with_commands(mut self, commands: impl IntoIterator<Item = CommandMeta>) -> Self {
        self.commands = commands.into_iter().collect();
        self
    }

    /// The tool contributions.
    #[must_use]
    pub fn with_tools(mut self, tools: impl IntoIterator<Item = Arc<dyn ToolPort>>) -> Self {
        self.tools = tools.into_iter().collect();
        self
    }

    /// The `@`-mention provider contributions.
    #[must_use]
    pub fn with_context_providers(
        mut self,
        providers: impl IntoIterator<Item = Arc<dyn ContextProviderPort>>,
    ) -> Self {
        self.providers = providers.into_iter().collect();
        self
    }

    /// The settings JSON Schema (`null` by default).
    #[must_use]
    pub fn with_settings_schema(mut self, schema: Value) -> Self {
        self.settings_schema = schema;
        self
    }
}

#[async_trait]
impl IntegrationPort for FakeIntegrationPort {
    fn id(&self) -> &str {
        &self.id
    }

    async fn detect(&self, session: &IntegrationSession) -> OxiResult<Capabilities> {
        self.calls.record(IntegrationCall::Detect(session.clone()));
        self.script.detect.next_or_else(|| Ok(self.capabilities))
    }

    fn sidebar(&self) -> SidebarModel {
        self.calls.record(IntegrationCall::Sidebar);
        self.sidebar.clone()
    }

    fn commands(&self) -> Vec<CommandMeta> {
        self.calls.record(IntegrationCall::Commands);
        self.commands.clone()
    }

    fn tools(&self) -> Vec<Arc<dyn ToolPort>> {
        self.calls.record(IntegrationCall::Tools);
        self.tools.clone()
    }

    fn context_providers(&self) -> Vec<Arc<dyn ContextProviderPort>> {
        self.calls.record(IntegrationCall::ContextProviders);
        self.providers.clone()
    }

    fn settings_schema(&self) -> Value {
        self.calls.record(IntegrationCall::SettingsSchema);
        self.settings_schema.clone()
    }
}

// --- ToolPort ----------------------------------------------------------------------------

/// Queued responses for [`FakeToolPort`].
#[derive(Debug, Default)]
pub struct ToolScripts {
    /// `invoke`.
    pub invoke: Script<ToolOutput>,
}

/// One call made on a [`FakeToolPort`] (`def()` is a plain accessor and is not recorded).
#[derive(Debug, Clone, PartialEq)]
pub enum ToolCall {
    /// `invoke(args, ctx)`.
    Invoke {
        /// Arguments passed.
        args: Value,
        /// Invocation context.
        ctx: ToolContext,
    },
}

/// Fake `ToolPort` with a fixed [`ToolDef`]. `invoke` is scripted only.
#[derive(Debug)]
pub struct FakeToolPort {
    script: ToolScripts,
    calls: CallLog<ToolCall>,
    def: ToolDef,
}

fake_plumbing!(FakeToolPort, ToolScripts, ToolCall);

impl FakeToolPort {
    /// A tool described by `def`.
    pub fn new(def: ToolDef) -> Self {
        Self {
            script: ToolScripts::default(),
            calls: CallLog::default(),
            def,
        }
    }
}

#[async_trait]
impl ToolPort for FakeToolPort {
    fn def(&self) -> &ToolDef {
        &self.def
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput> {
        self.calls.record(ToolCall::Invoke {
            args,
            ctx: ctx.clone(),
        });
        self.script
            .invoke
            .next_or_unscripted("FakeToolPort", "invoke")
    }
}

// --- ContextProviderPort -----------------------------------------------------------------

/// Queued responses for [`FakeContextProviderPort`].
#[derive(Debug, Default)]
pub struct ContextProviderScripts {
    /// `resolve`.
    pub resolve: Script<Vec<ContextBlock>>,
}

/// One call made on a [`FakeContextProviderPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextProviderCall {
    /// `resolve(mention, scope)`.
    Resolve {
        /// The mention resolved.
        mention: Mention,
        /// The scope passed.
        scope: ContextScope,
    },
}

/// Fake `ContextProviderPort` answering one mention prefix. `resolve` is scripted only;
/// `handles` keeps the trait's default (prefix match).
#[derive(Debug)]
pub struct FakeContextProviderPort {
    script: ContextProviderScripts,
    calls: CallLog<ContextProviderCall>,
    prefix: String,
}

fake_plumbing!(
    FakeContextProviderPort,
    ContextProviderScripts,
    ContextProviderCall
);

impl FakeContextProviderPort {
    /// A provider for `@prefix` mentions.
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            script: ContextProviderScripts::default(),
            calls: CallLog::default(),
            prefix: prefix.into(),
        }
    }
}

#[async_trait]
impl ContextProviderPort for FakeContextProviderPort {
    fn mention_prefix(&self) -> &str {
        &self.prefix
    }

    async fn resolve(
        &self,
        mention: &Mention,
        scope: &ContextScope,
    ) -> OxiResult<Vec<ContextBlock>> {
        self.calls.record(ContextProviderCall::Resolve {
            mention: mention.clone(),
            scope: scope.clone(),
        });
        self.script
            .resolve
            .next_or_unscripted("FakeContextProviderPort", "resolve")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use oxikube_domain::command::{CommandId, CommandScope};
    use oxikube_domain::ids::{ClusterId, ContextName};
    use oxikube_domain::session::ClusterSessionState;
    use oxikube_domain::{ErrorKind, Initiator, OxiError};
    use oxikube_ports::ToolName;
    use serde_json::json;

    fn session() -> IntegrationSession {
        let context = ContextName::new("kind-oxikube");
        IntegrationSession::new(
            ClusterId::new("/k", &context),
            context,
            ClusterSessionState::Ready,
            [],
        )
    }

    fn tool_def() -> ToolDef {
        ToolDef::read_only(
            ToolName::new("argo.list_apps").unwrap(),
            "List Argo CD applications",
            json!({"type": "object"}),
        )
    }

    #[test]
    fn integration_detect_scripts_and_getters_are_recorded() {
        let tool: Arc<dyn ToolPort> = Arc::new(FakeToolPort::new(tool_def()));
        let meta = CommandMeta::read(
            CommandId::new("argo::SyncStatus"),
            "Argo: sync status",
            CommandScope::Cluster,
            Capabilities::ARGO,
        );
        let fake = FakeIntegrationPort::new("argocd")
            .with_capabilities(Capabilities::ARGO)
            .with_commands([meta])
            .with_tools([tool])
            .with_settings_schema(json!({"type": "object"}));
        fake.script()
            .detect
            .push_ok(Capabilities::empty())
            .push_err(OxiError::forbidden("no CRD access"));
        assert_eq!(fake.id(), "argocd");
        assert_eq!(
            block_on(fake.detect(&session())).unwrap(),
            Capabilities::empty()
        );
        assert_eq!(
            block_on(fake.detect(&session())).unwrap_err().kind(),
            ErrorKind::Forbidden
        );
        assert_eq!(
            block_on(fake.detect(&session())).unwrap(),
            Capabilities::ARGO
        );
        assert_eq!(fake.commands(), vec![meta]);
        assert_eq!(fake.tools().len(), 1);
        assert!(fake.context_providers().is_empty());
        assert!(fake.sidebar().sections.is_empty());
        assert_eq!(fake.settings_schema(), json!({"type": "object"}));
        let calls = fake.recorded_calls();
        assert_eq!(calls.len(), 8);
        assert_eq!(calls[0], IntegrationCall::Detect(session()));
        assert_eq!(calls[7], IntegrationCall::SettingsSchema);
    }

    #[test]
    fn tool_scripted_ok_and_err_and_records_args() {
        let fake = FakeToolPort::new(tool_def());
        assert_eq!(fake.def().name.as_str(), "argo.list_apps");
        fake.script()
            .invoke
            .push_ok(ToolOutput::text("2 apps"))
            .push_err(OxiError::timeout("argo slow"));
        let ctx = ToolContext::new(Initiator::Agent);
        let out = block_on(fake.invoke(json!({"ns": "argocd"}), &ctx)).unwrap();
        assert_eq!(out.content[0].as_text(), Some("2 apps"));
        assert_eq!(
            block_on(fake.invoke(json!({}), &ctx)).unwrap_err().kind(),
            ErrorKind::Timeout
        );
        assert_eq!(
            block_on(fake.invoke(json!({}), &ctx)).unwrap_err().kind(),
            ErrorKind::Internal
        );
        assert_eq!(
            fake.recorded_calls()[0],
            ToolCall::Invoke {
                args: json!({"ns": "argocd"}),
                ctx
            }
        );
    }

    #[test]
    fn context_provider_scripted_ok_and_err_and_records_mentions() {
        let fake = FakeContextProviderPort::new("pod");
        let mention = Mention::parse("@pod/demo/web").unwrap();
        assert!(fake.handles(&mention));
        assert!(!fake.handles(&Mention::parse("@log/x").unwrap()));
        fake.script()
            .resolve
            .push_ok(vec![ContextBlock::text("pod demo/web", "Running")])
            .push_err(OxiError::not_found("no such pod"));
        let scope = ContextScope::new().with_default_namespace("demo");
        assert_eq!(block_on(fake.resolve(&mention, &scope)).unwrap().len(), 1);
        assert_eq!(
            block_on(fake.resolve(&mention, &scope)).unwrap_err().kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            fake.recorded_calls(),
            vec![ContextProviderCall::Resolve { mention, scope }; 2]
        );
    }
}
