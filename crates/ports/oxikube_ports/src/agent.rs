//! Hosted agents: [`AgentPort`] and the [`AgentClient`] callbacks.
//!
//! Oxikube is an ACP *client* (ADR 0008): it launches an agent process and talks
//! to it. The two traits split that relationship along who implements what.
//!
//! * [`AgentPort`] is the connection to one agent: the calls Oxikube makes
//!   (initialise, authenticate, new session, prompt, cancel, close) and a stream
//!   of [`AgentUpdateBatch`]es the agent pushes back. `oxikube_acp` implements
//!   it; `oxikube_app::AgentSessionManager` calls it.
//! * [`AgentClient`] is what the agent may ask *of* Oxikube: permission to run a
//!   tool call, `fs/*` reads and writes, `terminal/*`, elicitation.
//!   `oxikube_app` implements it (behind `MutationGuard` and the permission
//!   prompt) and hands it to [`AgentPort::initialize`]; `oxikube_acp` calls it.
//!
//! # Shaped after ACP, not copied from it
//!
//! The vocabulary follows the Agent Client Protocol 2.x client duties (reviewed
//! against `agent-client-protocol` 2.2.0, `schema::v1`), but the types are plain
//! and opaque ids replace the protocol's. ACP moved from 1.x to 2.x within a
//! month, so protocol enums are not mirrored one to one: the enums here are
//! `#[non_exhaustive]` and the adapter maps what it understands and drops what
//! it does not. No `agent-client-protocol` type appears in a signature.
//!
//! # ACP client duties and where they land
//!
//! | ACP (`schema::v1`) | Here |
//! |---|---|
//! | `initialize` (`InitializeRequest`/`Response`) | [`AgentPort::initialize`], [`ClientInfo`], [`ClientCapabilities`], [`AgentInfo`] |
//! | `authenticate` | [`AgentPort::authenticate`], [`AuthMethod`] |
//! | `session/new` (`NewSessionRequest`) | [`AgentPort::new_session`], [`NewSessionRequest`], [`McpServerConfig`] |
//! | `session/prompt` (`PromptRequest`/`Response`, `StopReason`) | [`AgentPort::prompt`], [`StopReason`] |
//! | `session/cancel` | [`AgentPort::cancel`] |
//! | `session/close` | [`AgentPort::close_session`] |
//! | `session/update` (`SessionNotification`): message, thought, tool call, tool call update, plan, session info | [`AgentPort::updates`], [`AgentUpdateBatch`], [`SessionUpdate`] |
//! | `session/request_permission` | [`AgentClient::request_permission`] |
//! | `fs/read_text_file`, `fs/write_text_file` | [`AgentClient::read_text_file`], [`AgentClient::write_text_file`] |
//! | `terminal/create`, `output`, `wait_for_exit`, `kill`, `release` | [`AgentClient::create_terminal`] and the four siblings |
//! | `elicitation/create` | [`AgentClient::elicit`] |
//! | `session/load`, `resume`, `fork`, `list`, `delete`, `set_mode`, `set_config_option`, providers, available commands, usage, notices, compaction | deferred: not needed for the first agent thread; add as `AgentPort` methods or `SessionUpdate` variants when a story needs them |
//!
//! # Batching
//!
//! Agents stream tokens. [`AgentPort::updates`] yields [`AgentUpdateBatch`]es,
//! each holding the updates the adapter coalesced since the last item (a
//! [`MessageChunk`] is a run of text, not a token), so the app notifies the UI
//! once per batch.
//!
//! # Safety
//!
//! The agent never mutates the cluster directly. A mutating tool call arrives
//! as MCP `ToolPort::invoke` (wrapped by `MutationGuard`, `Initiator::Agent`)
//! and the agent asks permission through [`AgentClient::request_permission`]
//! first. `fs/write_text_file` on a virtual `oxikube://` path becomes an editor
//! diff that applies only after the user confirms.

use std::fmt;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use oxikube_domain::{OxiError, OxiResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::ContentPart;

macro_rules! opaque_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Arc<str>);

        impl $name {
            /// Wraps an id string chosen by the agent or the app.
            pub fn new(id: impl Into<Arc<str>>) -> Self {
                Self(id.into())
            }

            /// The id text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self::new(id)
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                Self::new(id)
            }
        }
    };
}

opaque_id! {
    /// Opaque id of an agent session (an ACP `SessionId`). Not related to the
    /// cluster session state machine.
    AgentSessionId
}
opaque_id! {
    /// Opaque id of one tool call within a session (an ACP `ToolCallId`). Also
    /// threaded through `ToolContext` so an MCP invocation can be matched with
    /// the thread entry the agent announced.
    ToolCallId
}
opaque_id! {
    /// Opaque id of one option offered in a [`PermissionRequest`].
    PermissionOptionId
}
opaque_id! {
    /// Opaque id of a terminal created through [`AgentClient::create_terminal`].
    TerminalId
}
opaque_id! {
    /// Opaque id of an authentication method an agent offers.
    AuthMethodId
}

/// A stream of update batches from one agent connection. Ends when the agent
/// process exits or the connection closes; an `Err` item is a transport failure.
pub type AgentUpdateStream = Pin<Box<dyn Stream<Item = OxiResult<AgentUpdateBatch>> + Send>>;

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Who Oxikube says it is during [`AgentPort::initialize`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// Product name, `"oxikube"`.
    pub name: String,
    /// Product version.
    pub version: String,
}

/// What an [`AgentClient`] is willing to do. Advertised in
/// [`AgentPort::initialize`]; an agent must not call what is not advertised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClientCapabilities {
    /// `fs/read_text_file` is available.
    pub fs_read: bool,
    /// `fs/write_text_file` is available.
    pub fs_write: bool,
    /// The `terminal/*` methods are available.
    pub terminal: bool,
    /// Elicitation requests are available.
    pub elicitation: bool,
}

/// What the agent reports about itself after [`AgentPort::initialize`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentInfo {
    /// Agent name, for example `"claude-code"`.
    pub name: String,
    /// Agent version.
    pub version: String,
    /// A display title, when the agent supplies one.
    pub title: Option<String>,
    /// What the agent supports.
    pub capabilities: AgentCapabilities,
    /// Authentication methods; empty when no authentication is needed.
    pub auth_methods: Vec<AuthMethod>,
}

/// Features an agent supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AgentCapabilities {
    /// The agent can resume a stored session ([`AgentPort`] does not expose
    /// loading yet; see the module table in the story).
    pub load_session: bool,
    /// Prompts may contain [`ContentPart::Image`].
    pub prompt_images: bool,
    /// Prompts may contain [`ContentPart::Resource`] (embedded context).
    pub prompt_embedded_context: bool,
    /// The agent can connect to an MCP server over HTTP.
    pub mcp_http: bool,
}

/// One way to authenticate with an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthMethod {
    /// Id passed to [`AgentPort::authenticate`].
    pub id: AuthMethodId,
    /// Display name.
    pub name: String,
    /// Longer explanation for the user, when present.
    pub description: Option<String>,
}

/// Parameters for [`AgentPort::new_session`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSessionRequest {
    /// Working directory of the session. Agents resolve relative paths against it.
    pub cwd: PathBuf,
    /// MCP servers the agent should connect to (normally Oxikube's own
    /// `oxikube_mcp` server exposing the `ToolRegistry`).
    pub mcp_servers: Vec<McpServerConfig>,
}

/// An MCP server the agent is told to connect to.
///
/// `Debug` prints names only: environment values and header values can carry
/// tokens and must never reach a log (non-negotiable 5).
#[derive(Clone, PartialEq, Eq)]
pub enum McpServerConfig {
    /// A subprocess speaking MCP over stdio.
    Stdio {
        /// Display name.
        name: String,
        /// Executable path.
        command: PathBuf,
        /// Arguments.
        args: Vec<String>,
        /// Environment variables as `(name, value)`.
        env: Vec<(String, String)>,
    },
    /// A server reachable over streamable HTTP.
    Http {
        /// Display name.
        name: String,
        /// Endpoint URL.
        url: String,
        /// Request headers as `(name, value)`.
        headers: Vec<(String, String)>,
    },
}

impl fmt::Debug for McpServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stdio {
                name,
                command,
                args,
                env,
            } => f
                .debug_struct("Stdio")
                .field("name", name)
                .field("command", command)
                .field("args", &args.len())
                .field("env", &env.iter().map(|(k, _)| k).collect::<Vec<_>>())
                .finish(),
            Self::Http { name, url, headers } => f
                .debug_struct("Http")
                .field("name", name)
                .field("url", url)
                .field(
                    "headers",
                    &headers.iter().map(|(k, _)| k).collect::<Vec<_>>(),
                )
                .finish(),
        }
    }
}

/// Why a prompt turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StopReason {
    /// The agent finished its turn.
    EndTurn,
    /// The token limit was reached.
    MaxTokens,
    /// The agent hit its per-turn request limit.
    MaxTurnRequests,
    /// The agent refused to continue.
    Refusal,
    /// [`AgentPort::cancel`] ended the turn.
    Cancelled,
}

/// One connection to a hosted agent. Implemented by `oxikube_acp`.
///
/// All methods take `&self`: [`prompt`](Self::prompt) runs for the whole turn
/// while [`cancel`](Self::cancel) and permission answers arrive concurrently.
/// Dropping a returned future abandons that call but does not cancel the turn;
/// call [`cancel`](Self::cancel).
///
/// # Effects
///
/// Not a cluster mutation: the cluster-touching calls an agent makes come back through
/// [`AgentClient`] and `MutationGuard`.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) when the agent process or its
/// transport dies, [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) for a feature the
/// agent did not advertise in [`AgentCapabilities`],
/// [`NotFound`](oxikube_domain::ErrorKind::NotFound) for an unknown [`AgentSessionId`],
/// [`Internal`](oxikube_domain::ErrorKind::Internal) for a protocol violation.
#[async_trait]
pub trait AgentPort: Send + Sync {
    /// Handshake: sends `client_info` and the capabilities of `client`, and
    /// keeps `client` to answer the agent's callbacks for the lifetime of the
    /// connection. Call exactly once, first; a second call returns
    /// [`Conflict`](oxikube_domain::ErrorKind::Conflict).
    async fn initialize(
        &self,
        client_info: ClientInfo,
        client: Arc<dyn AgentClient>,
    ) -> OxiResult<AgentInfo>;

    /// Authenticates with one of [`AgentInfo::auth_methods`]. Fails with
    /// [`Auth`](oxikube_domain::ErrorKind::Auth) when rejected.
    async fn authenticate(&self, method: &AuthMethodId) -> OxiResult<()>;

    /// Opens a conversation and returns its id.
    async fn new_session(&self, request: NewSessionRequest) -> OxiResult<AgentSessionId>;

    /// Sends a user turn and resolves when the turn ends. Streamed output
    /// arrives on [`updates`](Self::updates) before this returns.
    async fn prompt(
        &self,
        session: &AgentSessionId,
        prompt: Vec<ContentPart>,
    ) -> OxiResult<StopReason>;

    /// Asks the agent to stop the current turn. The pending
    /// [`prompt`](Self::prompt) resolves with [`StopReason::Cancelled`].
    /// Cancelling an idle session is a no-op.
    async fn cancel(&self, session: &AgentSessionId) -> OxiResult<()>;

    /// Closes a session and frees the agent's resources for it.
    async fn close_session(&self, session: &AgentSessionId) -> OxiResult<()>;

    /// The stream of updates for every session on this connection, coalesced
    /// into batches. Each call returns a new stream that sees updates from now
    /// on; an adapter that supports one subscriber ends the earlier stream.
    fn updates(&self) -> AgentUpdateStream;
}

// ---------------------------------------------------------------------------
// Session updates
// ---------------------------------------------------------------------------

/// Updates for one session, coalesced by the adapter, oldest first.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentUpdateBatch {
    /// The session the updates belong to.
    pub session: AgentSessionId,
    /// The updates, in the order the agent sent them.
    pub updates: Vec<SessionUpdate>,
}

/// A coalesced run of streamed text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageChunk {
    /// The content, usually [`ContentPart::Text`].
    pub content: ContentPart,
}

impl MessageChunk {
    /// A text chunk.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: ContentPart::text(text),
        }
    }
}

/// One thing that happened in a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "update", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionUpdate {
    /// The agent is writing its reply.
    AgentMessage(MessageChunk),
    /// The agent is reasoning (shown collapsed).
    AgentThought(MessageChunk),
    /// The agent started a tool call.
    ToolCall(ToolCallInfo),
    /// Progress on an announced tool call.
    ToolCallUpdate(ToolCallPatch),
    /// The agent replaced its plan.
    Plan(Plan),
    /// The agent renamed the session.
    TitleChanged {
        /// The new title.
        title: String,
    },
}

/// What a tool call does, for the icon and grouping in the thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolKind {
    /// Reads data.
    Read,
    /// Changes a file or object.
    Edit,
    /// Deletes something.
    Delete,
    /// Moves or renames something.
    Move,
    /// Searches.
    Search,
    /// Runs a command.
    Execute,
    /// Reasons internally.
    Think,
    /// Fetches from the network.
    Fetch,
    /// Anything else.
    #[default]
    Other,
}

/// Where a tool call is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolCallStatus {
    /// Announced, not started (possibly awaiting permission).
    #[default]
    Pending,
    /// Running.
    InProgress,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
}

/// Content attached to a tool call in the thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolCallContent {
    /// Ordinary content.
    Content {
        /// The content part.
        content: ContentPart,
    },
    /// A proposed or applied file change.
    Diff {
        /// The path (a virtual `oxikube://` path for cluster objects).
        path: PathBuf,
        /// Previous text; `None` for a new file.
        old_text: Option<String>,
        /// New text.
        new_text: String,
    },
    /// A live terminal created through [`AgentClient::create_terminal`].
    Terminal {
        /// The terminal's id.
        terminal_id: TerminalId,
    },
}

/// A tool call as first announced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallInfo {
    /// The call's id.
    pub id: ToolCallId,
    /// Human title, for example `Scale deployment web to 3`.
    pub title: String,
    /// What the call does.
    pub kind: ToolKind,
    /// Current status.
    pub status: ToolCallStatus,
    /// Attached content so far.
    pub content: Vec<ToolCallContent>,
    /// The raw input arguments, when the agent sends them.
    pub raw_input: Option<Value>,
    /// The raw output, when available.
    pub raw_output: Option<Value>,
}

impl ToolCallInfo {
    /// A pending call with no content.
    pub fn new(id: ToolCallId, title: impl Into<String>, kind: ToolKind) -> Self {
        Self {
            id,
            title: title.into(),
            kind,
            status: ToolCallStatus::Pending,
            content: Vec::new(),
            raw_input: None,
            raw_output: None,
        }
    }

    /// Applies `patch`: each field the patch sets replaces the field here.
    pub fn apply(&mut self, patch: &ToolCallPatch) {
        if let Some(title) = &patch.title {
            self.title.clone_from(title);
        }
        if let Some(kind) = patch.kind {
            self.kind = kind;
        }
        if let Some(status) = patch.status {
            self.status = status;
        }
        if let Some(content) = &patch.content {
            self.content.clone_from(content);
        }
        if let Some(raw_input) = &patch.raw_input {
            self.raw_input = Some(raw_input.clone());
        }
        if let Some(raw_output) = &patch.raw_output {
            self.raw_output = Some(raw_output.clone());
        }
    }
}

/// A partial change to a [`ToolCallInfo`]. `None` leaves a field as it was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallPatch {
    /// The call being updated.
    pub id: ToolCallId,
    /// New title.
    pub title: Option<String>,
    /// New kind.
    pub kind: Option<ToolKind>,
    /// New status.
    pub status: Option<ToolCallStatus>,
    /// Replacement content (the whole list, not an append).
    pub content: Option<Vec<ToolCallContent>>,
    /// New raw input.
    pub raw_input: Option<Value>,
    /// New raw output.
    pub raw_output: Option<Value>,
}

impl ToolCallPatch {
    /// A patch for call `id` that changes nothing yet.
    pub fn new(id: ToolCallId) -> Self {
        Self {
            id,
            title: None,
            kind: None,
            status: None,
            content: None,
            raw_input: None,
            raw_output: None,
        }
    }

    /// A patch for call `id` that sets only the status.
    pub fn status(id: ToolCallId, status: ToolCallStatus) -> Self {
        Self {
            status: Some(status),
            ..Self::new(id)
        }
    }
}

/// The agent's plan for the current task.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Plan {
    /// The steps; a new plan replaces the previous one.
    pub entries: Vec<PlanEntry>,
}

/// One step of a [`Plan`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    /// What the step does.
    pub content: String,
    /// How important it is.
    pub priority: PlanPriority,
    /// Where it stands.
    pub status: PlanStatus,
}

/// Importance of a [`PlanEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PlanPriority {
    /// Low.
    Low,
    /// Medium.
    Medium,
    /// High.
    High,
}

/// Progress of a [`PlanEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PlanStatus {
    /// Not started.
    Pending,
    /// Being worked on.
    InProgress,
    /// Done.
    Completed,
}

// ---------------------------------------------------------------------------
// Client callbacks
// ---------------------------------------------------------------------------

/// What the agent asks permission for.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionRequest {
    /// The session asking.
    pub session: AgentSessionId,
    /// The tool call needing permission.
    pub tool_call: ToolCallPatch,
    /// The choices to show, in the agent's order.
    pub options: Vec<PermissionOption>,
}

/// One choice in a [`PermissionRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionOption {
    /// Id returned in [`PermissionOutcome::Selected`].
    pub id: PermissionOptionId,
    /// Button label.
    pub name: String,
    /// What choosing it means.
    pub kind: PermissionOptionKind,
}

/// The meaning of a [`PermissionOption`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PermissionOptionKind {
    /// Allow this call.
    AllowOnce,
    /// Allow this and similar calls for the rest of the session.
    AllowAlways,
    /// Reject this call.
    RejectOnce,
    /// Reject this and similar calls for the rest of the session.
    RejectAlways,
}

impl PermissionOptionKind {
    /// Whether choosing this kind lets the call run.
    pub const fn allows(self) -> bool {
        matches!(self, Self::AllowOnce | Self::AllowAlways)
    }
}

/// The user's answer to a [`PermissionRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PermissionOutcome {
    /// The prompt was dismissed or the turn was cancelled; treat as a reject.
    Cancelled,
    /// The user picked this option.
    Selected(PermissionOptionId),
}

/// Parameters of `fs/read_text_file`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadTextFile {
    /// The session reading.
    pub session: AgentSessionId,
    /// File path; a virtual `oxikube://` path arrives verbatim.
    pub path: PathBuf,
    /// 1-based first line to return.
    pub line: Option<u32>,
    /// Maximum number of lines.
    pub limit: Option<u32>,
}

/// Parameters of `fs/write_text_file`.
///
/// Writing to a virtual `oxikube://` path opens an editor diff and applies
/// only after the user confirms (ADR 0008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteTextFile {
    /// The session writing.
    pub session: AgentSessionId,
    /// File path.
    pub path: PathBuf,
    /// The full new content.
    pub content: String,
}

/// Parameters of `terminal/create`.
#[derive(Clone, PartialEq, Eq)]
pub struct CreateTerminal {
    /// The session asking.
    pub session: AgentSessionId,
    /// Executable to run.
    pub command: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Extra environment as `(name, value)`.
    pub env: Vec<(String, String)>,
    /// Working directory.
    pub cwd: Option<PathBuf>,
    /// Keep at most this many bytes of output, dropping the oldest.
    pub output_byte_limit: Option<u64>,
}

impl fmt::Debug for CreateTerminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateTerminal")
            .field("session", &self.session)
            .field("command", &self.command)
            .field("args", &self.args.len())
            .field("env", &self.env.iter().map(|(k, _)| k).collect::<Vec<_>>())
            .field("cwd", &self.cwd)
            .field("output_byte_limit", &self.output_byte_limit)
            .finish()
    }
}

/// How a terminal command ended.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalExit {
    /// Exit code, when the process exited normally.
    pub exit_code: Option<u32>,
    /// Terminating signal name, when it was killed by one.
    pub signal: Option<String>,
}

/// Output captured from a terminal so far.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalOutput {
    /// Captured output (already redacted by the implementer).
    pub output: String,
    /// Whether the start of the output was dropped to respect the byte limit.
    pub truncated: bool,
    /// Set once the command has exited.
    pub exit: Option<TerminalExit>,
}

/// An agent's request for user input outside a permission prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct ElicitationRequest {
    /// The session asking, when it is session-scoped.
    pub session: Option<AgentSessionId>,
    /// The question or instruction to show.
    pub message: String,
    /// What to collect.
    pub mode: ElicitationMode,
}

/// What an [`ElicitationRequest`] collects.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ElicitationMode {
    /// A form. `schema` is a flat JSON schema object (`properties` of string,
    /// number, integer, boolean and enum fields); the answer is a JSON object.
    Form {
        /// The JSON schema of the requested fields.
        schema: Value,
    },
    /// Ask the user to complete something at a URL (for example an OAuth flow).
    Url {
        /// The URL to open, after the user agrees.
        url: String,
    },
}

/// The user's answer to an [`ElicitationRequest`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ElicitationResponse {
    /// The user supplied the data (a JSON object for a form; `Null` for a URL).
    Accept(Value),
    /// The user said no.
    Decline,
    /// The user dismissed the prompt.
    Cancel,
}

fn unsupported(method: &str) -> OxiError {
    OxiError::unsupported(format!("agent client callback {method} is not supported"))
}

/// What the agent may ask of Oxikube. Implemented by `oxikube_app`, called by
/// `oxikube_acp`.
///
/// Only [`request_permission`](Self::request_permission) is required. Every
/// other callback defaults to
/// [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) and is advertised
/// through [`capabilities`](Self::capabilities); override both together.
///
/// Implementations must never persist or log file contents, terminal output or
/// form answers (non-negotiable 5), and must run anything that touches the
/// cluster through `MutationGuard` with `Initiator::Agent`.
#[async_trait]
pub trait AgentClient: Send + Sync {
    /// Which optional callbacks this client implements. Defaults to none.
    fn capabilities(&self) -> ClientCapabilities {
        ClientCapabilities::default()
    }

    /// Asks the user whether a tool call may run. Resolves when the user
    /// answers or the turn is cancelled ([`PermissionOutcome::Cancelled`]).
    async fn request_permission(&self, request: PermissionRequest) -> OxiResult<PermissionOutcome>;

    /// `fs/read_text_file`: returns the file text.
    async fn read_text_file(&self, request: ReadTextFile) -> OxiResult<String> {
        let _ = request;
        Err(unsupported("fs/read_text_file"))
    }

    /// `fs/write_text_file`: writes the file, or opens a confirmation diff for
    /// a virtual path and resolves after the user decides.
    async fn write_text_file(&self, request: WriteTextFile) -> OxiResult<()> {
        let _ = request;
        Err(unsupported("fs/write_text_file"))
    }

    /// `terminal/create`: starts a command and returns immediately with its id.
    async fn create_terminal(&self, request: CreateTerminal) -> OxiResult<TerminalId> {
        let _ = request;
        Err(unsupported("terminal/create"))
    }

    /// `terminal/output`: output captured so far, without waiting.
    async fn terminal_output(
        &self,
        session: &AgentSessionId,
        terminal: &TerminalId,
    ) -> OxiResult<TerminalOutput> {
        let _ = (session, terminal);
        Err(unsupported("terminal/output"))
    }

    /// `terminal/wait_for_exit`: resolves when the command exits.
    async fn wait_for_terminal_exit(
        &self,
        session: &AgentSessionId,
        terminal: &TerminalId,
    ) -> OxiResult<TerminalExit> {
        let _ = (session, terminal);
        Err(unsupported("terminal/wait_for_exit"))
    }

    /// `terminal/kill`: kills the command but keeps the terminal for output.
    async fn kill_terminal(
        &self,
        session: &AgentSessionId,
        terminal: &TerminalId,
    ) -> OxiResult<()> {
        let _ = (session, terminal);
        Err(unsupported("terminal/kill"))
    }

    /// `terminal/release`: kills the command if running and frees the terminal.
    async fn release_terminal(
        &self,
        session: &AgentSessionId,
        terminal: &TerminalId,
    ) -> OxiResult<()> {
        let _ = (session, terminal);
        Err(unsupported("terminal/release"))
    }

    /// Elicitation: asks the user for input and resolves with their answer.
    async fn elicit(&self, request: ElicitationRequest) -> OxiResult<ElicitationResponse> {
        let _ = request;
        Err(unsupported("elicitation"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_domain::ErrorKind;
    use serde_json::json;

    #[test]
    fn ids_are_opaque_and_serialise_as_plain_strings() {
        let id = AgentSessionId::new("sess_1");
        assert_eq!(id.as_str(), "sess_1");
        assert_eq!(id.to_string(), "sess_1");
        assert_eq!(serde_json::to_value(&id).unwrap(), json!("sess_1"));
        let back: AgentSessionId = serde_json::from_value(json!("sess_1")).unwrap();
        assert_eq!(back, id);
        assert_eq!(ToolCallId::from("c1"), ToolCallId::from("c1".to_owned()));
    }

    #[test]
    fn patch_updates_only_the_fields_it_sets() {
        let mut call = ToolCallInfo::new(ToolCallId::from("c1"), "Scale web", ToolKind::Edit);
        assert_eq!(call.status, ToolCallStatus::Pending);
        call.apply(&ToolCallPatch::status(
            ToolCallId::from("c1"),
            ToolCallStatus::InProgress,
        ));
        assert_eq!(call.status, ToolCallStatus::InProgress);
        assert_eq!(call.title, "Scale web");

        call.apply(&ToolCallPatch {
            title: Some("Scaled web".into()),
            content: Some(vec![ToolCallContent::Content {
                content: ContentPart::text("ok"),
            }]),
            raw_output: Some(json!({"replicas": 3})),
            ..ToolCallPatch::new(ToolCallId::from("c1"))
        });
        assert_eq!(call.title, "Scaled web");
        assert_eq!(call.content.len(), 1);
        assert_eq!(call.raw_output, Some(json!({"replicas": 3})));
        assert_eq!(call.kind, ToolKind::Edit);
    }

    #[test]
    fn tool_call_content_round_trips_every_variant() {
        let variants = vec![
            ToolCallContent::Content {
                content: ContentPart::text("x"),
            },
            ToolCallContent::Diff {
                path: PathBuf::from("oxikube://deploy/web"),
                old_text: Some("a".into()),
                new_text: "b".into(),
            },
            ToolCallContent::Terminal {
                terminal_id: TerminalId::from("t1"),
            },
        ];
        for variant in variants {
            let v = serde_json::to_value(&variant).unwrap();
            assert!(v["type"].is_string(), "{v}");
            assert_eq!(
                serde_json::from_value::<ToolCallContent>(v.clone()).unwrap(),
                variant,
                "{v}"
            );
        }
        let v = serde_json::to_value(ToolCallContent::Content {
            content: ContentPart::text("x"),
        })
        .unwrap();
        assert_eq!(v["type"], "content");
        assert_eq!(v["content"]["type"], "text");

        let mut call = ToolCallInfo::new(ToolCallId::from("c1"), "t", ToolKind::Edit);
        call.content.push(ToolCallContent::Terminal {
            terminal_id: TerminalId::from("t1"),
        });
        let update = SessionUpdate::ToolCall(call);
        let v = serde_json::to_value(&update).unwrap();
        assert_eq!(serde_json::from_value::<SessionUpdate>(v).unwrap(), update);
    }

    #[test]
    fn session_update_serde_is_tagged() {
        let u = SessionUpdate::AgentMessage(MessageChunk::text("hi"));
        let v = serde_json::to_value(&u).unwrap();
        assert_eq!(v["update"], "agent_message");
        assert_eq!(serde_json::from_value::<SessionUpdate>(v).unwrap(), u);

        let u = SessionUpdate::Plan(Plan {
            entries: vec![PlanEntry {
                content: "scale".into(),
                priority: PlanPriority::High,
                status: PlanStatus::InProgress,
            }],
        });
        let v = serde_json::to_value(&u).unwrap();
        assert_eq!(serde_json::from_value::<SessionUpdate>(v).unwrap(), u);
    }

    #[test]
    fn permission_kinds_say_whether_they_allow() {
        assert!(PermissionOptionKind::AllowOnce.allows());
        assert!(PermissionOptionKind::AllowAlways.allows());
        assert!(!PermissionOptionKind::RejectOnce.allows());
        assert!(!PermissionOptionKind::RejectAlways.allows());
    }

    #[test]
    fn debug_hides_secret_values() {
        let cfg = McpServerConfig::Http {
            name: "oxikube".into(),
            url: "http://127.0.0.1:1".into(),
            headers: vec![("Authorization".into(), "Bearer hunter2".into())],
        };
        let shown = format!("{cfg:?}");
        assert!(shown.contains("Authorization") && !shown.contains("hunter2"));

        let cfg = McpServerConfig::Stdio {
            name: "oxikube".into(),
            command: "oxikube".into(),
            args: vec![],
            env: vec![("TOKEN".into(), "hunter2".into())],
        };
        assert!(!format!("{cfg:?}").contains("hunter2"));

        let term = CreateTerminal {
            session: AgentSessionId::from("s"),
            command: "env".into(),
            args: vec![],
            env: vec![("TOKEN".into(), "hunter2".into())],
            cwd: None,
            output_byte_limit: None,
        };
        assert!(!format!("{term:?}").contains("hunter2"));
    }

    struct OnlyPermission;

    #[async_trait]
    impl AgentClient for OnlyPermission {
        async fn request_permission(&self, _: PermissionRequest) -> OxiResult<PermissionOutcome> {
            Ok(PermissionOutcome::Cancelled)
        }
    }

    #[test]
    fn optional_callbacks_default_to_unsupported() {
        let client: Arc<dyn AgentClient> = Arc::new(OnlyPermission);
        assert_eq!(client.capabilities(), ClientCapabilities::default());
        let session = AgentSessionId::from("s");
        let err = futures::executor::block_on(client.read_text_file(ReadTextFile {
            session: session.clone(),
            path: "/x".into(),
            line: None,
            limit: None,
        }))
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unsupported);
        let err =
            futures::executor::block_on(client.kill_terminal(&session, &TerminalId::from("t")))
                .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unsupported);
        let err = futures::executor::block_on(client.elicit(ElicitationRequest {
            session: None,
            message: "?".into(),
            mode: ElicitationMode::Url {
                url: "https://x".into(),
            },
        }))
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unsupported);
    }
}
