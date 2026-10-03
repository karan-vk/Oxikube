//! Compile-time proof that the integration and agent ports are object-safe and
//! usable without `rmcp` or `agent-client-protocol`: each trait is implemented
//! by a small stub, stored as `Arc<dyn Trait>`, called through the trait object
//! with a `Send` future, and exercised end to end (detect, invoke, resolve, and
//! a full agent session with streamed updates and client callbacks).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::StreamExt;
use futures::executor::block_on;
use oxikube_domain::agent::ContextBlock;
use oxikube_domain::command::{CommandId, CommandMeta, CommandScope};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::session::ClusterSessionState;
use oxikube_domain::{Capabilities, ErrorKind, Initiator, OxiError, OxiResult, Risk};
use oxikube_ports::*;
use serde_json::{Value, json};

fn assert_send<T: Send>(t: T) -> T {
    t
}

// --- ToolPort ---------------------------------------------------------------

struct ScaleTool(ToolDef);

impl ScaleTool {
    fn new() -> Self {
        Self(ToolDef::mutating(
            ToolName::new("k8s.workload_scale").unwrap(),
            "Scale a workload",
            json!({"type": "object", "properties": {"replicas": {"type": "integer"}}}),
            Risk::Medium,
        ))
    }
}

#[async_trait]
impl ToolPort for ScaleTool {
    fn def(&self) -> &ToolDef {
        &self.0
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput> {
        if ctx.initiator != Initiator::Agent {
            return Err(OxiError::forbidden("agents only"));
        }
        match args.get("replicas").and_then(Value::as_i64) {
            Some(n) => Ok(ToolOutput::json(json!({ "replicas": n }))),
            None => Ok(ToolOutput::error("replicas is required")),
        }
    }
}

#[test]
fn tool_port_is_object_safe() {
    let tool: Arc<dyn ToolPort> = Arc::new(ScaleTool::new());
    assert!(tool.def().is_mutating());
    tool.def().validate().unwrap();

    let ctx = ToolContext::agent().with_cluster(cluster());
    let out = block_on(assert_send(tool.invoke(json!({"replicas": 3}), &ctx))).unwrap();
    assert_eq!(out.structured, Some(json!({"replicas": 3})));
    assert!(!out.is_error);

    let out = block_on(tool.invoke(json!({}), &ctx)).unwrap();
    assert!(out.is_error);

    let err = block_on(tool.invoke(json!({}), &ToolContext::new(Initiator::Ui))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}

// --- ContextProviderPort ----------------------------------------------------

struct PodContext;

#[async_trait]
impl ContextProviderPort for PodContext {
    fn mention_prefix(&self) -> &str {
        "pod"
    }

    async fn resolve(
        &self,
        mention: &Mention,
        scope: &ContextScope,
    ) -> OxiResult<Vec<ContextBlock>> {
        let (ns, name) = match mention.path() {
            [ns, name] => (ns.as_str(), name.as_str()),
            [name] => (
                scope
                    .default_namespace
                    .as_deref()
                    .ok_or_else(|| OxiError::validation("namespace required"))?,
                name.as_str(),
            ),
            _ => return Err(OxiError::validation("expected @pod/<ns>/<name>")),
        };
        Ok(vec![ContextBlock::bounded(
            format!("Pod {ns}/{name}"),
            "application/yaml",
            "kind: Pod\n",
        )])
    }
}

#[test]
fn context_provider_port_is_object_safe() {
    let provider: Arc<dyn ContextProviderPort> = Arc::new(PodContext);
    let scope = ContextScope::new().with_default_namespace("default");

    let m = Mention::parse("@pod/default/web-0").unwrap();
    assert!(provider.handles(&m));
    let blocks = block_on(assert_send(provider.resolve(&m, &scope))).unwrap();
    assert_eq!(blocks[0].title, "Pod default/web-0");

    let m = Mention::parse("@pod/web-0").unwrap();
    assert_eq!(
        block_on(provider.resolve(&m, &scope)).unwrap()[0].title,
        "Pod default/web-0"
    );
    let err = block_on(provider.resolve(&m, &ContextScope::new())).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);

    let other = Mention::parse("@logs/default/web-0").unwrap();
    assert!(!provider.handles(&other));
}

// --- IntegrationPort --------------------------------------------------------

const ARGO_REFRESH: CommandMeta = CommandMeta::read(
    CommandId::new("argo::HardRefresh"),
    "Hard Refresh",
    CommandScope::Selection,
    Capabilities::ARGO,
);

struct ArgoLike;

fn application() -> Gvk {
    Gvk::from_api_version("argoproj.io/v1alpha1", "Application")
}

#[async_trait]
impl IntegrationPort for ArgoLike {
    fn id(&self) -> &str {
        "argocd"
    }

    async fn detect(&self, session: &IntegrationSession) -> OxiResult<Capabilities> {
        Ok(
            if session.is_connected() && session.serves(&application()) {
                Capabilities::ARGO
            } else {
                Capabilities::empty()
            },
        )
    }

    fn sidebar(&self) -> SidebarModel {
        SidebarModel::single(SidebarSection {
            id: "argo".into(),
            title: "Argo CD".into(),
            icon: Some("git-branch".into()),
            items: vec![SidebarItem {
                id: "refresh".into(),
                title: "Hard Refresh".into(),
                icon: None,
                command: ARGO_REFRESH.id,
                needs: Capabilities::ARGO,
            }],
        })
    }

    fn commands(&self) -> Vec<CommandMeta> {
        vec![ARGO_REFRESH]
    }

    fn tools(&self) -> Vec<Arc<dyn ToolPort>> {
        vec![Arc::new(ScaleTool::new())]
    }

    fn context_providers(&self) -> Vec<Arc<dyn ContextProviderPort>> {
        vec![Arc::new(PodContext)]
    }

    fn settings_schema(&self) -> Value {
        json!({"type": "object", "properties": {"server": {"type": "string"}}})
    }
}

fn cluster() -> ClusterId {
    ClusterId::new("kubeconfig", &ContextName::new("kind-oxikube"))
}

fn session(state: ClusterSessionState, kinds: impl IntoIterator<Item = Gvk>) -> IntegrationSession {
    IntegrationSession {
        cluster: cluster(),
        context: ContextName::new("kind-oxikube"),
        state,
        served_kinds: kinds.into_iter().collect::<BTreeSet<_>>(),
    }
}

#[test]
fn integration_port_is_object_safe() {
    let integration: Arc<dyn IntegrationPort> = Arc::new(ArgoLike);
    assert_eq!(integration.id(), "argocd");

    let present = session(ClusterSessionState::Ready, [application()]);
    let caps = block_on(assert_send(integration.detect(&present))).unwrap();
    assert_eq!(caps, Capabilities::ARGO);

    let absent = session(ClusterSessionState::Ready, []);
    assert!(block_on(integration.detect(&absent)).unwrap().is_empty());
    let offline = session(ClusterSessionState::Disconnected, [application()]);
    assert!(block_on(integration.detect(&offline)).unwrap().is_empty());

    // Everything the integration contributes is plain data or another port.
    assert_eq!(integration.sidebar().visible_with(caps).sections.len(), 1);
    assert!(
        integration
            .sidebar()
            .visible_with(Capabilities::empty())
            .sections
            .is_empty()
    );
    assert_eq!(
        integration.commands()[0].id,
        CommandId::new("argo::HardRefresh")
    );
    assert_eq!(integration.tools()[0].def().name.namespace(), "k8s");
    assert_eq!(integration.context_providers()[0].mention_prefix(), "pod");
    assert_eq!(integration.settings_schema()["type"], "object");
}

// --- AgentPort + AgentClient ------------------------------------------------

/// What the stub agent saw, for assertions.
#[derive(Default)]
struct Seen {
    initialised: bool,
    cancelled: Vec<AgentSessionId>,
    closed: Vec<AgentSessionId>,
    prompts: Vec<Vec<ContentPart>>,
}

struct StubAgent {
    seen: Mutex<Seen>,
    client: Mutex<Option<Arc<dyn AgentClient>>>,
}

impl StubAgent {
    fn new() -> Self {
        Self {
            seen: Mutex::new(Seen::default()),
            client: Mutex::new(None),
        }
    }
}

#[async_trait]
impl AgentPort for StubAgent {
    async fn initialize(
        &self,
        client_info: ClientInfo,
        client: Arc<dyn AgentClient>,
    ) -> OxiResult<AgentInfo> {
        let mut seen = self.seen.lock().unwrap();
        if seen.initialised {
            return Err(OxiError::conflict("already initialised"));
        }
        seen.initialised = true;
        *self.client.lock().unwrap() = Some(client);
        Ok(AgentInfo {
            name: "stub-agent".into(),
            version: client_info.version,
            capabilities: AgentCapabilities {
                prompt_embedded_context: true,
                ..AgentCapabilities::default()
            },
            auth_methods: vec![AuthMethod {
                id: AuthMethodId::from("token"),
                name: "Token".into(),
                description: None,
            }],
            ..AgentInfo::default()
        })
    }

    async fn authenticate(&self, method: &AuthMethodId) -> OxiResult<()> {
        if method.as_str() == "token" {
            Ok(())
        } else {
            Err(OxiError::auth("unknown method", false))
        }
    }

    async fn new_session(&self, request: NewSessionRequest) -> OxiResult<AgentSessionId> {
        assert!(request.cwd.is_absolute());
        Ok(AgentSessionId::from("sess_1"))
    }

    async fn prompt(
        &self,
        session: &AgentSessionId,
        prompt: Vec<ContentPart>,
    ) -> OxiResult<StopReason> {
        self.seen.lock().unwrap().prompts.push(prompt);
        // The agent asks permission through the client mid-turn.
        let client = self.client.lock().unwrap().clone().expect("initialised");
        let outcome = client
            .request_permission(PermissionRequest {
                session: session.clone(),
                tool_call: ToolCallPatch::status(
                    ToolCallId::from("call_1"),
                    ToolCallStatus::Pending,
                ),
                options: vec![
                    PermissionOption {
                        id: PermissionOptionId::from("allow"),
                        name: "Allow".into(),
                        kind: PermissionOptionKind::AllowOnce,
                    },
                    PermissionOption {
                        id: PermissionOptionId::from("reject"),
                        name: "Reject".into(),
                        kind: PermissionOptionKind::RejectOnce,
                    },
                ],
            })
            .await?;
        Ok(match outcome {
            PermissionOutcome::Selected(id) if id.as_str() == "allow" => StopReason::EndTurn,
            _ => StopReason::Refusal,
        })
    }

    async fn cancel(&self, session: &AgentSessionId) -> OxiResult<()> {
        self.seen.lock().unwrap().cancelled.push(session.clone());
        Ok(())
    }

    async fn close_session(&self, session: &AgentSessionId) -> OxiResult<()> {
        self.seen.lock().unwrap().closed.push(session.clone());
        Ok(())
    }

    fn updates(&self) -> AgentUpdateStream {
        let session = AgentSessionId::from("sess_1");
        let call = ToolCallInfo::new(ToolCallId::from("call_1"), "Scale web", ToolKind::Edit);
        futures::stream::iter(vec![
            Ok(AgentUpdateBatch {
                session: session.clone(),
                updates: vec![
                    SessionUpdate::AgentThought(MessageChunk::text("thinking")),
                    SessionUpdate::AgentMessage(MessageChunk::text("Scaling web to 3.")),
                    SessionUpdate::ToolCall(call),
                ],
            }),
            Ok(AgentUpdateBatch {
                session,
                updates: vec![SessionUpdate::ToolCallUpdate(ToolCallPatch::status(
                    ToolCallId::from("call_1"),
                    ToolCallStatus::Completed,
                ))],
            }),
            Err(OxiError::network("agent exited")),
        ])
        .boxed()
    }
}

/// Answers permission prompts and serves files from memory.
struct StubClient {
    allow: bool,
    written: Mutex<Vec<(std::path::PathBuf, String)>>,
}

#[async_trait]
impl AgentClient for StubClient {
    fn capabilities(&self) -> ClientCapabilities {
        ClientCapabilities {
            fs_read: true,
            fs_write: true,
            terminal: false,
            elicitation: true,
        }
    }

    async fn request_permission(&self, request: PermissionRequest) -> OxiResult<PermissionOutcome> {
        let wanted = if self.allow {
            PermissionOptionKind::AllowOnce
        } else {
            PermissionOptionKind::RejectOnce
        };
        Ok(request
            .options
            .iter()
            .find(|o| o.kind == wanted)
            .map_or(PermissionOutcome::Cancelled, |o| {
                PermissionOutcome::Selected(o.id.clone())
            }))
    }

    async fn read_text_file(&self, request: ReadTextFile) -> OxiResult<String> {
        Ok(format!("contents of {}", request.path.display()))
    }

    async fn write_text_file(&self, request: WriteTextFile) -> OxiResult<()> {
        self.written
            .lock()
            .unwrap()
            .push((request.path, request.content));
        Ok(())
    }

    async fn elicit(&self, request: ElicitationRequest) -> OxiResult<ElicitationResponse> {
        Ok(match request.mode {
            ElicitationMode::Form { .. } => ElicitationResponse::Accept(json!({"ok": true})),
            _ => ElicitationResponse::Decline,
        })
    }
}

fn client(allow: bool) -> Arc<StubClient> {
    Arc::new(StubClient {
        allow,
        written: Mutex::new(Vec::new()),
    })
}

fn info() -> ClientInfo {
    ClientInfo {
        name: "oxikube".into(),
        version: "0.0.1".into(),
    }
}

#[test]
fn agent_port_full_session_lifecycle_through_trait_objects() {
    let agent: Arc<dyn AgentPort> = Arc::new(StubAgent::new());
    let client: Arc<dyn AgentClient> = client(true);

    let agent_info = block_on(assert_send(agent.initialize(info(), client.clone()))).unwrap();
    assert!(agent_info.capabilities.prompt_embedded_context);
    assert_eq!(
        block_on(agent.initialize(info(), client))
            .unwrap_err()
            .kind(),
        ErrorKind::Conflict
    );

    block_on(agent.authenticate(&agent_info.auth_methods[0].id)).unwrap();
    assert_eq!(
        block_on(agent.authenticate(&AuthMethodId::from("nope")))
            .unwrap_err()
            .kind(),
        ErrorKind::Auth
    );

    let request = NewSessionRequest {
        cwd: "/work".into(),
        mcp_servers: vec![McpServerConfig::Stdio {
            name: "oxikube".into(),
            command: "oxikube".into(),
            args: vec!["mcp".into()],
            env: vec![],
        }],
    };
    let session = block_on(assert_send(agent.new_session(request))).unwrap();

    let prompt = vec![
        ContentPart::text("Why is web-0 crashing?"),
        ContentPart::from(ContextBlock::text("Pod default/web-0", "kind: Pod")),
    ];
    let stop = block_on(assert_send(agent.prompt(&session, prompt))).unwrap();
    assert_eq!(stop, StopReason::EndTurn);

    // The stream yields coalesced batches, then a transport error.
    let items: Vec<_> = block_on(agent.updates().collect());
    assert_eq!(items.len(), 3);
    let first = items[0].as_ref().unwrap();
    assert_eq!(first.updates.len(), 3);
    assert!(matches!(first.updates[2], SessionUpdate::ToolCall(_)));
    assert_eq!(items[2].as_ref().unwrap_err().kind(), ErrorKind::Network);

    block_on(agent.cancel(&session)).unwrap();
    block_on(agent.close_session(&session)).unwrap();
}

#[test]
fn rejected_permission_ends_the_turn_with_a_refusal() {
    let agent: Arc<dyn AgentPort> = Arc::new(StubAgent::new());
    block_on(agent.initialize(info(), client(false))).unwrap();
    let session = block_on(agent.new_session(NewSessionRequest {
        cwd: "/work".into(),
        mcp_servers: vec![],
    }))
    .unwrap();
    let stop = block_on(agent.prompt(&session, vec![ContentPart::text("delete it")])).unwrap();
    assert_eq!(stop, StopReason::Refusal);
}

#[test]
fn agent_client_callbacks_are_object_safe() {
    let concrete = client(true);
    let client: Arc<dyn AgentClient> = concrete.clone();
    let session = AgentSessionId::from("sess_1");
    assert!(client.capabilities().fs_read && !client.capabilities().terminal);

    let text = block_on(assert_send(client.read_text_file(ReadTextFile {
        session: session.clone(),
        path: "oxikube://pod/default/web-0".into(),
        line: None,
        limit: None,
    })))
    .unwrap();
    assert!(text.contains("oxikube://pod/default/web-0"));

    block_on(assert_send(client.write_text_file(WriteTextFile {
        session: session.clone(),
        path: "oxikube://pod/default/web-0".into(),
        content: "kind: Pod\n".into(),
    })))
    .unwrap();
    assert_eq!(concrete.written.lock().unwrap().len(), 1);

    // `terminal/*` was not advertised, so the defaults answer Unsupported.
    let err = block_on(assert_send(client.create_terminal(CreateTerminal {
        session: session.clone(),
        command: "ls".into(),
        args: vec![],
        env: vec![],
        cwd: None,
        output_byte_limit: None,
    })))
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    let terminal = TerminalId::from("t1");
    for r in [
        block_on(client.kill_terminal(&session, &terminal)).map(|_| ()),
        block_on(client.release_terminal(&session, &terminal)).map(|_| ()),
        block_on(client.terminal_output(&session, &terminal)).map(|_| ()),
        block_on(client.wait_for_terminal_exit(&session, &terminal)).map(|_| ()),
    ] {
        assert_eq!(r.unwrap_err().kind(), ErrorKind::Unsupported);
    }

    let answer = block_on(assert_send(client.elicit(ElicitationRequest {
        session: Some(session),
        message: "Which namespace?".into(),
        mode: ElicitationMode::Form {
            schema: json!({"type": "object", "properties": {"ns": {"type": "string"}}}),
        },
    })))
    .unwrap();
    assert_eq!(answer, ElicitationResponse::Accept(json!({"ok": true})));
}

#[test]
fn the_four_ports_can_share_one_registry_vec() {
    // The registries in oxikube_app hold these as trait objects side by side.
    let _: Vec<Arc<dyn IntegrationPort>> = vec![Arc::new(ArgoLike)];
    let _: Vec<Arc<dyn ToolPort>> = vec![Arc::new(ScaleTool::new())];
    let _: Vec<Arc<dyn ContextProviderPort>> = vec![Arc::new(PodContext)];
    let _: Vec<Arc<dyn AgentPort>> = vec![Arc::new(StubAgent::new())];
}
