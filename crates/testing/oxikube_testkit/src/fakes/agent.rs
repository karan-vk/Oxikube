//! Agent fakes: [`FakeAgentPort`] (an ACP agent as the app sees it) and
//! [`FakeAgentClient`] (the app's callbacks as an agent adapter sees them).

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use futures::channel::mpsc;
use oxikube_domain::OxiResult;
use oxikube_ports::{
    AgentClient, AgentInfo, AgentPort, AgentSessionId, AgentUpdateBatch, AgentUpdateStream,
    AuthMethodId, ClientCapabilities, ClientInfo, ClockPort, ContentPart, CreateTerminal,
    ElicitationRequest, ElicitationResponse, NewSessionRequest, PermissionOutcome,
    PermissionRequest, ReadTextFile, StopReason, TerminalExit, TerminalId, TerminalOutput,
    WriteTextFile,
};
use parking_lot::Mutex;

use super::FakeClockPort;
use crate::script::{CallLog, Script, Timeline};

// --- AgentPort ---------------------------------------------------------------------------

/// Queued responses for each [`FakeAgentPort`] method.
#[derive(Debug, Default)]
pub struct AgentScripts {
    /// `initialize`.
    pub initialize: Script<AgentInfo>,
    /// `authenticate`.
    pub authenticate: Script<()>,
    /// `new_session`.
    pub new_session: Script<AgentSessionId>,
    /// `prompt`.
    pub prompt: Script<StopReason>,
    /// `cancel`.
    pub cancel: Script<()>,
    /// `close_session`.
    pub close_session: Script<()>,
    /// `updates`: each entry is the timeline one update stream replays (an `Err` entry
    /// becomes a stream that yields that error and ends).
    pub updates: Script<Timeline<AgentUpdateBatch>>,
}

/// One call made on a [`FakeAgentPort`].
#[derive(Debug, Clone, PartialEq)]
pub enum AgentCall {
    /// `initialize(client_info, _)`.
    Initialize(ClientInfo),
    /// `authenticate(method)`.
    Authenticate(AuthMethodId),
    /// `new_session(request)`.
    NewSession(NewSessionRequest),
    /// `prompt(session, prompt)`.
    Prompt {
        /// Session prompted.
        session: AgentSessionId,
        /// Prompt content.
        prompt: Vec<ContentPart>,
    },
    /// `cancel(session)`.
    Cancel(AgentSessionId),
    /// `close_session(session)`.
    CloseSession(AgentSessionId),
    /// `updates()`.
    Updates,
}

/// Fake `AgentPort`.
///
/// Fallbacks: `initialize` returns the configured [`AgentInfo`] and keeps the client
/// (see [`client`](Self::client)); `new_session` hands out `session-1`, `session-2`, ...;
/// `prompt` ends with `EndTurn`; `authenticate`, `cancel` and `close_session` succeed.
/// `updates` replays a scripted [`Timeline`] on [`clock`](Self::clock), or, when nothing
/// is scripted, returns a live stream fed by [`send_update`](Self::send_update).
pub struct FakeAgentPort {
    script: AgentScripts,
    calls: CallLog<AgentCall>,
    info: AgentInfo,
    client: Mutex<Option<Arc<dyn AgentClient>>>,
    sessions: Mutex<u64>,
    live: Mutex<Vec<mpsc::UnboundedSender<OxiResult<AgentUpdateBatch>>>>,
    clock: Arc<FakeClockPort>,
}

fake_plumbing!(FakeAgentPort, AgentScripts, AgentCall);

impl Default for FakeAgentPort {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for FakeAgentPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeAgentPort")
            .field("info", &self.info)
            .field("initialized", &self.client.lock().is_some())
            .field("calls", &self.calls.len())
            .finish_non_exhaustive()
    }
}

impl FakeAgentPort {
    /// A fake agent named `fake-agent` with default capabilities and its own clock.
    pub fn new() -> Self {
        Self::with_clock(Arc::new(FakeClockPort::default()))
    }

    /// A fake agent whose scripted update timelines replay on `clock`.
    pub fn with_clock(clock: Arc<FakeClockPort>) -> Self {
        Self {
            script: AgentScripts::default(),
            calls: CallLog::default(),
            info: AgentInfo {
                name: "fake-agent".into(),
                version: "0.0.0".into(),
                ..AgentInfo::default()
            },
            client: Mutex::new(None),
            sessions: Mutex::new(0),
            live: Mutex::new(Vec::new()),
            clock,
        }
    }

    /// The [`AgentInfo`] `initialize` returns when nothing is scripted.
    #[must_use]
    pub fn with_info(mut self, info: AgentInfo) -> Self {
        self.info = info;
        self
    }

    /// The clock scripted update timelines replay on.
    pub fn clock(&self) -> &Arc<FakeClockPort> {
        &self.clock
    }

    /// The client passed to the last `initialize`, so a test can drive the callbacks an
    /// agent would make (permission requests, file reads).
    pub fn client(&self) -> Option<Arc<dyn AgentClient>> {
        self.client.lock().clone()
    }

    /// Pushes `batch` to every live (unscripted) `updates` stream. Returns how many
    /// streams received it.
    pub fn send_update(&self, batch: AgentUpdateBatch) -> usize {
        let mut live = self.live.lock();
        live.retain(|tx| tx.unbounded_send(Ok(batch.clone())).is_ok());
        live.len()
    }

    /// Ends every live `updates` stream.
    pub fn close_updates(&self) {
        self.live.lock().clear();
    }
}

#[async_trait]
impl AgentPort for FakeAgentPort {
    async fn initialize(
        &self,
        client_info: ClientInfo,
        client: Arc<dyn AgentClient>,
    ) -> OxiResult<AgentInfo> {
        self.calls.record(AgentCall::Initialize(client_info));
        *self.client.lock() = Some(client);
        self.script
            .initialize
            .next_or_else(|| Ok(self.info.clone()))
    }

    async fn authenticate(&self, method: &AuthMethodId) -> OxiResult<()> {
        self.calls.record(AgentCall::Authenticate(method.clone()));
        self.script.authenticate.next_or_else(|| Ok(()))
    }

    async fn new_session(&self, request: NewSessionRequest) -> OxiResult<AgentSessionId> {
        self.calls.record(AgentCall::NewSession(request));
        self.script.new_session.next_or_else(|| {
            let mut n = self.sessions.lock();
            *n += 1;
            Ok(AgentSessionId::new(format!("session-{n}")))
        })
    }

    async fn prompt(
        &self,
        session: &AgentSessionId,
        prompt: Vec<ContentPart>,
    ) -> OxiResult<StopReason> {
        self.calls.record(AgentCall::Prompt {
            session: session.clone(),
            prompt,
        });
        self.script.prompt.next_or_else(|| Ok(StopReason::EndTurn))
    }

    async fn cancel(&self, session: &AgentSessionId) -> OxiResult<()> {
        self.calls.record(AgentCall::Cancel(session.clone()));
        self.script.cancel.next_or_else(|| Ok(()))
    }

    async fn close_session(&self, session: &AgentSessionId) -> OxiResult<()> {
        self.calls.record(AgentCall::CloseSession(session.clone()));
        self.script.close_session.next_or_else(|| Ok(()))
    }

    fn updates(&self) -> AgentUpdateStream {
        self.calls.record(AgentCall::Updates);
        match self.script.updates.pop() {
            Some(Ok(timeline)) => {
                let clock: Arc<dyn ClockPort> = self.clock.clone();
                timeline.replay(clock)
            }
            Some(Err(error)) => futures::stream::iter([Err(error)]).boxed(),
            None => {
                let (tx, rx) = mpsc::unbounded();
                self.live.lock().push(tx);
                rx.boxed()
            }
        }
    }
}

// --- AgentClient -------------------------------------------------------------------------

/// Queued responses for each [`FakeAgentClient`] callback.
#[derive(Debug, Default)]
pub struct AgentClientScripts {
    /// `request_permission`.
    pub request_permission: Script<PermissionOutcome>,
    /// `read_text_file`.
    pub read_text_file: Script<String>,
    /// `write_text_file`.
    pub write_text_file: Script<()>,
    /// `create_terminal`.
    pub create_terminal: Script<TerminalId>,
    /// `terminal_output`.
    pub terminal_output: Script<TerminalOutput>,
    /// `wait_for_terminal_exit`.
    pub wait_for_terminal_exit: Script<TerminalExit>,
    /// `kill_terminal`.
    pub kill_terminal: Script<()>,
    /// `release_terminal`.
    pub release_terminal: Script<()>,
    /// `elicit`.
    pub elicit: Script<ElicitationResponse>,
}

/// One callback made on a [`FakeAgentClient`] (`capabilities()` is not recorded).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentClientCall {
    /// `request_permission`.
    RequestPermission(PermissionRequest),
    /// `read_text_file`.
    ReadTextFile(ReadTextFile),
    /// `write_text_file`.
    WriteTextFile(WriteTextFile),
    /// `create_terminal`.
    CreateTerminal(CreateTerminal),
    /// `terminal_output`.
    TerminalOutput(AgentSessionId, TerminalId),
    /// `wait_for_terminal_exit`.
    WaitForTerminalExit(AgentSessionId, TerminalId),
    /// `kill_terminal`.
    KillTerminal(AgentSessionId, TerminalId),
    /// `release_terminal`.
    ReleaseTerminal(AgentSessionId, TerminalId),
    /// `elicit`.
    Elicit(ElicitationRequest),
}

/// Fake `AgentClient` for agent-adapter tests. Every callback is scripted only; the
/// advertised capabilities are configured with [`with_capabilities`](Self::with_capabilities)
/// (all off by default).
#[derive(Debug, Default)]
pub struct FakeAgentClient {
    script: AgentClientScripts,
    calls: CallLog<AgentClientCall>,
    capabilities: ClientCapabilities,
}

fake_plumbing!(FakeAgentClient, AgentClientScripts, AgentClientCall);

impl FakeAgentClient {
    /// A client that advertises no optional capability.
    pub fn new() -> Self {
        Self::default()
    }

    /// The capabilities `capabilities()` advertises.
    #[must_use]
    pub fn with_capabilities(mut self, capabilities: ClientCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }
}

const CLIENT: &str = "FakeAgentClient";

#[async_trait]
impl AgentClient for FakeAgentClient {
    fn capabilities(&self) -> ClientCapabilities {
        self.capabilities
    }

    async fn request_permission(&self, request: PermissionRequest) -> OxiResult<PermissionOutcome> {
        self.calls
            .record(AgentClientCall::RequestPermission(request));
        self.script
            .request_permission
            .next_or_unscripted(CLIENT, "request_permission")
    }

    async fn read_text_file(&self, request: ReadTextFile) -> OxiResult<String> {
        self.calls.record(AgentClientCall::ReadTextFile(request));
        self.script
            .read_text_file
            .next_or_unscripted(CLIENT, "read_text_file")
    }

    async fn write_text_file(&self, request: WriteTextFile) -> OxiResult<()> {
        self.calls.record(AgentClientCall::WriteTextFile(request));
        self.script
            .write_text_file
            .next_or_unscripted(CLIENT, "write_text_file")
    }

    async fn create_terminal(&self, request: CreateTerminal) -> OxiResult<TerminalId> {
        self.calls.record(AgentClientCall::CreateTerminal(request));
        self.script
            .create_terminal
            .next_or_unscripted(CLIENT, "create_terminal")
    }

    async fn terminal_output(
        &self,
        session: &AgentSessionId,
        terminal: &TerminalId,
    ) -> OxiResult<TerminalOutput> {
        self.calls.record(AgentClientCall::TerminalOutput(
            session.clone(),
            terminal.clone(),
        ));
        self.script
            .terminal_output
            .next_or_unscripted(CLIENT, "terminal_output")
    }

    async fn wait_for_terminal_exit(
        &self,
        session: &AgentSessionId,
        terminal: &TerminalId,
    ) -> OxiResult<TerminalExit> {
        self.calls.record(AgentClientCall::WaitForTerminalExit(
            session.clone(),
            terminal.clone(),
        ));
        self.script
            .wait_for_terminal_exit
            .next_or_unscripted(CLIENT, "wait_for_terminal_exit")
    }

    async fn kill_terminal(
        &self,
        session: &AgentSessionId,
        terminal: &TerminalId,
    ) -> OxiResult<()> {
        self.calls.record(AgentClientCall::KillTerminal(
            session.clone(),
            terminal.clone(),
        ));
        self.script
            .kill_terminal
            .next_or_unscripted(CLIENT, "kill_terminal")
    }

    async fn release_terminal(
        &self,
        session: &AgentSessionId,
        terminal: &TerminalId,
    ) -> OxiResult<()> {
        self.calls.record(AgentClientCall::ReleaseTerminal(
            session.clone(),
            terminal.clone(),
        ));
        self.script
            .release_terminal
            .next_or_unscripted(CLIENT, "release_terminal")
    }

    async fn elicit(&self, request: ElicitationRequest) -> OxiResult<ElicitationResponse> {
        self.calls.record(AgentClientCall::Elicit(request));
        self.script.elicit.next_or_unscripted(CLIENT, "elicit")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use futures::FutureExt;
    use futures::executor::block_on;
    use oxikube_domain::{ErrorKind, OxiError};
    use oxikube_ports::{
        MessageChunk, PermissionOptionId, SessionUpdate, ToolCallId, ToolCallPatch,
    };

    fn client_info() -> ClientInfo {
        ClientInfo {
            name: "oxikube".into(),
            version: "0.1.0".into(),
        }
    }

    fn batch(session: &str, text: &str) -> AgentUpdateBatch {
        AgentUpdateBatch {
            session: AgentSessionId::new(session),
            updates: vec![SessionUpdate::AgentMessage(MessageChunk::text(text))],
        }
    }

    #[test]
    fn agent_lifecycle_falls_back_and_scripts_errors() {
        let fake = FakeAgentPort::new();
        let client: Arc<dyn AgentClient> = Arc::new(FakeAgentClient::new());
        let info = block_on(fake.initialize(client_info(), client)).unwrap();
        assert_eq!(info.name, "fake-agent");
        assert!(fake.client().is_some());
        let request = NewSessionRequest {
            cwd: PathBuf::from("/work"),
            mcp_servers: Vec::new(),
        };
        let s1 = block_on(fake.new_session(request.clone())).unwrap();
        let s2 = block_on(fake.new_session(request.clone())).unwrap();
        assert_eq!((s1.as_str(), s2.as_str()), ("session-1", "session-2"));

        fake.script()
            .prompt
            .push_ok(StopReason::Cancelled)
            .push_err(OxiError::auth("expired", false));
        let hello = vec![ContentPart::text("hello")];
        assert_eq!(
            block_on(fake.prompt(&s1, hello.clone())).unwrap(),
            StopReason::Cancelled
        );
        assert_eq!(
            block_on(fake.prompt(&s1, hello.clone()))
                .unwrap_err()
                .kind(),
            ErrorKind::Auth
        );
        assert_eq!(
            block_on(fake.prompt(&s1, hello.clone())).unwrap(),
            StopReason::EndTurn
        );
        block_on(fake.cancel(&s1)).unwrap();
        block_on(fake.close_session(&s1)).unwrap();
        block_on(fake.authenticate(&AuthMethodId::new("token"))).unwrap();

        let calls = fake.recorded_calls();
        assert_eq!(calls[0], AgentCall::Initialize(client_info()));
        assert_eq!(calls[1], AgentCall::NewSession(request));
        assert_eq!(
            calls[3],
            AgentCall::Prompt {
                session: s1,
                prompt: hello
            }
        );
        assert_eq!(calls.len(), 9);
    }

    #[test]
    fn agent_updates_replay_scripts_or_stream_live() {
        let fake = FakeAgentPort::new();
        fake.script()
            .updates
            .push_ok(Timeline::new().ok_at(Duration::from_secs(1), batch("s", "thinking")))
            .push_err(OxiError::network("agent exited"));
        let mut scripted = fake.updates();
        assert!(scripted.next().now_or_never().is_none());
        fake.clock().advance(Duration::from_secs(1));
        assert_eq!(
            block_on(scripted.next()).unwrap().unwrap(),
            batch("s", "thinking")
        );
        assert!(block_on(scripted.next()).is_none());

        let mut failed = fake.updates();
        assert!(block_on(failed.next()).unwrap().is_err());
        assert!(block_on(failed.next()).is_none());

        let mut live = fake.updates();
        assert_eq!(fake.send_update(batch("s", "hi")), 1);
        assert_eq!(block_on(live.next()).unwrap().unwrap(), batch("s", "hi"));
        fake.close_updates();
        assert!(block_on(live.next()).is_none());
        assert_eq!(fake.recorded_calls(), vec![AgentCall::Updates; 3]);
    }

    #[test]
    fn agent_client_scripts_callbacks_and_records_them() {
        let fake = FakeAgentClient::new().with_capabilities(ClientCapabilities {
            fs_read: true,
            ..ClientCapabilities::default()
        });
        assert!(fake.capabilities().fs_read);
        let session = AgentSessionId::new("s");
        let request = PermissionRequest {
            session: session.clone(),
            tool_call: ToolCallPatch::new(ToolCallId::new("t1")),
            options: Vec::new(),
        };
        fake.script()
            .request_permission
            .push_ok(PermissionOutcome::Selected(PermissionOptionId::new(
                "allow",
            )))
            .push_err(OxiError::internal("ui closed"));
        assert_eq!(
            block_on(fake.request_permission(request.clone())).unwrap(),
            PermissionOutcome::Selected(PermissionOptionId::new("allow"))
        );
        assert_eq!(
            block_on(fake.request_permission(request.clone()))
                .unwrap_err()
                .kind(),
            ErrorKind::Internal
        );
        let read = ReadTextFile {
            session: session.clone(),
            path: PathBuf::from("/work/a.yaml"),
            line: None,
            limit: None,
        };
        fake.script().read_text_file.push_ok("a: 1".into());
        assert_eq!(block_on(fake.read_text_file(read.clone())).unwrap(), "a: 1");
        let terminal = TerminalId::new("term-1");
        assert!(block_on(fake.kill_terminal(&session, &terminal)).is_err());
        assert_eq!(
            fake.recorded_calls(),
            vec![
                AgentClientCall::RequestPermission(request.clone()),
                AgentClientCall::RequestPermission(request),
                AgentClientCall::ReadTextFile(read),
                AgentClientCall::KillTerminal(session, terminal),
            ]
        );
    }
}
